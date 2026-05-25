//! CI equivalence harness for the orbit-knowledge v1 and orbit-graph v2 backends.

mod backend;

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use backend::{
    Backend, CalleeEntry, CalleesOutput, ImpactOutput, RefEntry, RefsOutput, SearchEntry,
    SearchOutput, ShowOutput, V1Backend, V2Backend,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const EXPECTED_CORPUS_SHA256: &str =
    "3e6d500f59c30707240791ac8e617cdb1a5f77dae08f2f431bec5eacca42eda7";
const LANGUAGES: [&str; 4] = ["rust", "typescript", "python", "go"];
const IMPACT_DEPTH: u8 = 3;

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "{error}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<(), Box<dyn Error>> {
    let options = match Options::parse(env::args().skip(1))? {
        ParsedOptions::Help => {
            write_usage()?;
            return Ok(());
        }
        ParsedOptions::Run(options) => options,
    };

    let corpus = Corpus::load(&options.corpus_dir)?;
    if corpus.checksum != options.expected_corpus_sha256 {
        return Err(HarnessError::CorpusDrift {
            expected: options.expected_corpus_sha256,
            actual: corpus.checksum,
            corpus_dir: options.corpus_dir,
        }
        .into());
    }

    let v1 = V1Backend::for_workspace(
        options.workspace_root.clone(),
        options.knowledge_dir.clone(),
    )?;
    let v2 = V2Backend::for_workspace(options.workspace_root.clone(), options.graph_cli.clone())?;
    v1.sync()?;
    v2.sync()?;

    let results = corpus
        .queries
        .iter()
        .map(|query| run_query(&v1, &v2, query))
        .collect::<Vec<_>>();
    let failed = results
        .iter()
        .filter(|result| result.status == QueryStatus::Fail)
        .count();
    let report = DiffReport {
        schema_version: 1,
        workspace: options.workspace_root.display().to_string(),
        corpus: CorpusReport {
            path: options.corpus_dir.display().to_string(),
            checksum: corpus.checksum,
            entries: corpus.queries.len(),
        },
        summary: Summary {
            total: results.len(),
            passed: results.len().saturating_sub(failed),
            failed,
        },
        results,
    };

    write_json(&report)?;
    if report.summary.failed > 0 {
        return Err(HarnessError::ToleranceViolations(report.summary.failed).into());
    }
    Ok(())
}

fn run_query(v1: &dyn Backend, v2: &dyn Backend, query: &CorpusQuery) -> QueryReport {
    match query.kind {
        QueryKind::Search => compare_backend_outputs(query, || {
            let v1_rows = v1.search(query.argument.as_str())?;
            let v2_rows = v2.search(query.argument.as_str())?;
            Ok(compare_search(v1_rows, v2_rows))
        }),
        QueryKind::Show => compare_backend_outputs(query, || {
            let v1_rows = v1.show(query.argument.as_str())?;
            let v2_rows = v2.show(query.argument.as_str())?;
            Ok(compare_show(v1_rows, v2_rows))
        }),
        QueryKind::Refs => compare_backend_outputs(query, || {
            let v1_rows = v1.refs(query.argument.as_str())?;
            let v2_rows = v2.refs(query.argument.as_str())?;
            Ok(compare_refs(v1_rows, v2_rows))
        }),
        QueryKind::Callees => compare_backend_outputs(query, || {
            let v1_rows = v1.callees(query.argument.as_str())?;
            let v2_rows = v2.callees(query.argument.as_str())?;
            Ok(compare_callees(v1_rows, v2_rows))
        }),
        QueryKind::Impact => compare_backend_outputs(query, || {
            let v1_rows = v1.impact(query.argument.as_str(), IMPACT_DEPTH)?;
            let v2_rows = v2.impact(query.argument.as_str(), IMPACT_DEPTH)?;
            Ok(compare_impact(v1_rows, v2_rows))
        }),
    }
}

fn compare_backend_outputs<F>(query: &CorpusQuery, run: F) -> QueryReport
where
    F: FnOnce() -> Result<Comparison, backend::BackendError>,
{
    match run() {
        Ok(comparison) => {
            let status = if comparison.violations.is_empty() {
                QueryStatus::Pass
            } else {
                QueryStatus::Fail
            };
            QueryReport::from_comparison(query, status, comparison)
        }
        Err(error) => QueryReport::from_comparison(
            query,
            QueryStatus::Fail,
            Comparison {
                tolerance: query.kind.tolerance().to_string(),
                v1_count: 0,
                v2_count: 0,
                ignored_v2_count: 0,
                violations: vec![Violation {
                    kind: "backend_error".to_string(),
                    rows: json!([{ "error": error.to_string() }]),
                }],
            },
        ),
    }
}

fn compare_search(v1: SearchOutput, v2: SearchOutput) -> Comparison {
    let v1_rows = v1.into_iter().map(SearchRow::from).collect::<BTreeSet<_>>();
    let mut ignored_v2_count = 0usize;
    let v2_rows = v2
        .into_iter()
        .filter_map(|entry| {
            if entry.kind == "symbol" {
                Some(SearchRow::from(entry))
            } else {
                ignored_v2_count += 1;
                None
            }
        })
        .collect::<BTreeSet<_>>();
    set_comparison(
        "search: unordered set of (kind,file,name); v2 string/config extras ignored",
        v1_rows,
        v2_rows,
        ignored_v2_count,
    )
}

fn compare_show(v1: ShowOutput, v2: ShowOutput) -> Comparison {
    let mut comparison = Comparison::new("show: source bytes byte-equal");
    comparison.v1_count = usize::from(v1.is_some());
    comparison.v2_count = usize::from(v2.is_some());
    if v1 != v2 {
        comparison.violations.push(Violation {
            kind: "bytes_mismatch".to_string(),
            rows: json!([
                show_digest("v1", v1.as_deref()),
                show_digest("v2", v2.as_deref()),
            ]),
        });
    }
    comparison
}

fn compare_refs(v1: RefsOutput, v2: RefsOutput) -> Comparison {
    let v1_rows = v1.into_iter().map(RefRow::from).collect::<BTreeSet<_>>();
    let v2_rows = v2.into_iter().map(RefRow::from).collect::<BTreeSet<_>>();
    set_comparison(
        "refs: set of (file,line,kind) at confidence >= same_module",
        v1_rows,
        v2_rows,
        0,
    )
}

fn compare_callees(v1: CalleesOutput, v2: CalleesOutput) -> Comparison {
    let v1_rows = v1.into_iter().map(CalleeRow::from).collect::<BTreeSet<_>>();
    let v2_rows = v2.into_iter().map(CalleeRow::from).collect::<BTreeSet<_>>();
    set_comparison(
        "callees: set of (file,line,target_name)",
        v1_rows,
        v2_rows,
        0,
    )
}

fn compare_impact(v1: ImpactOutput, v2: ImpactOutput) -> Comparison {
    let v1_rows = v1.into_iter().collect::<BTreeSet<_>>();
    let v2_rows = v2.into_iter().collect::<BTreeSet<_>>();
    set_comparison(
        "impact: depth=3 set of touched symbol qualified names",
        v1_rows,
        v2_rows,
        0,
    )
}

fn set_comparison<T>(
    tolerance: &str,
    v1_rows: BTreeSet<T>,
    v2_rows: BTreeSet<T>,
    ignored_v2_count: usize,
) -> Comparison
where
    T: Clone + Ord + Serialize,
{
    let missing = v1_rows.difference(&v2_rows).cloned().collect::<Vec<_>>();
    let extra = v2_rows.difference(&v1_rows).cloned().collect::<Vec<_>>();
    let mut comparison = Comparison::new(tolerance);
    comparison.v1_count = v1_rows.len();
    comparison.v2_count = v2_rows.len();
    comparison.ignored_v2_count = ignored_v2_count;
    if !missing.is_empty() {
        comparison.violations.push(Violation {
            kind: "missing_in_v2".to_string(),
            rows: json!(missing),
        });
    }
    if !extra.is_empty() {
        comparison.violations.push(Violation {
            kind: "extra_in_v2".to_string(),
            rows: json!(extra),
        });
    }
    comparison
}

fn show_digest(backend: &str, bytes: Option<&[u8]>) -> Value {
    let mut hasher = Sha256::new();
    if let Some(bytes) = bytes {
        hasher.update(bytes);
    }
    json!({
        "backend": backend,
        "present": bytes.is_some(),
        "len": bytes.map_or(0, <[u8]>::len),
        "sha256": format!("{:x}", hasher.finalize()),
    })
}

fn write_json<T: Serialize>(value: &T) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn write_usage() -> Result<(), Box<dyn Error>> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(
        b"usage: graph-equiv [check] [--workspace PATH] [--corpus PATH] [--knowledge-dir PATH] [--orbit-graph-cli PATH]\n",
    )?;
    stdout.flush()?;
    Ok(())
}

#[derive(Debug)]
struct Options {
    workspace_root: PathBuf,
    corpus_dir: PathBuf,
    knowledge_dir: Option<PathBuf>,
    graph_cli: Option<PathBuf>,
    expected_corpus_sha256: String,
}

enum ParsedOptions {
    Help,
    Run(Options),
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Result<ParsedOptions, Box<dyn Error>> {
        let mut workspace_root = env::current_dir()?;
        let mut corpus_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus");
        let mut knowledge_dir = None;
        let mut graph_cli = None;
        let mut expected_corpus_sha256 = EXPECTED_CORPUS_SHA256.to_string();
        let mut args = args.peekable();

        if args.peek().is_some_and(|arg| arg == "check") {
            let _ = args.next();
        }

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--workspace" => workspace_root = PathBuf::from(required_value(&mut args, &arg)?),
                "--corpus" => corpus_dir = PathBuf::from(required_value(&mut args, &arg)?),
                "--knowledge-dir" => {
                    knowledge_dir = Some(PathBuf::from(required_value(&mut args, &arg)?));
                }
                "--orbit-graph-cli" => {
                    graph_cli = Some(PathBuf::from(required_value(&mut args, &arg)?));
                }
                "--expected-corpus-sha256" => {
                    expected_corpus_sha256 = required_value(&mut args, &arg)?;
                }
                "--help" | "-h" => return Ok(ParsedOptions::Help),
                other => {
                    return Err(HarnessError::Input(format!("unknown option `{other}`")).into());
                }
            }
        }

        Ok(ParsedOptions::Run(Options {
            workspace_root,
            corpus_dir,
            knowledge_dir,
            graph_cli,
            expected_corpus_sha256,
        }))
    }
}

fn required_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, HarnessError> {
    args.next()
        .ok_or_else(|| HarnessError::Input(format!("missing value for `{flag}`")))
}

#[derive(Debug)]
struct Corpus {
    checksum: String,
    queries: Vec<CorpusQuery>,
}

impl Corpus {
    fn load(corpus_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let mut hasher = Sha256::new();
        let mut queries = Vec::new();
        for language in LANGUAGES {
            let path = corpus_dir.join(format!("{language}.txt"));
            let bytes = fs::read(path.as_path()).map_err(|source| HarnessError::ReadCorpus {
                path: path.clone(),
                source,
            })?;
            hasher.update(language.as_bytes());
            hasher.update([0]);
            hasher.update(&bytes);
            hasher.update([0xff]);

            let text = String::from_utf8(bytes).map_err(|source| HarnessError::CorpusUtf8 {
                path: path.clone(),
                source,
            })?;
            queries.extend(parse_corpus_file(language, path.as_path(), text.as_str())?);
        }

        Ok(Self {
            checksum: format!("{:x}", hasher.finalize()),
            queries,
        })
    }
}

fn parse_corpus_file(
    language: &str,
    path: &Path,
    text: &str,
) -> Result<Vec<CorpusQuery>, HarnessError> {
    let mut queries = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let kind = parts
            .next()
            .and_then(QueryKind::parse)
            .ok_or_else(|| HarnessError::Input(format!("invalid query kind in {path:?}:{line}")))?;
        let argument = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                HarnessError::Input(format!(
                    "missing query argument in {}:{line}",
                    path.display()
                ))
            })?;
        queries.push(CorpusQuery {
            language: language.to_string(),
            file: path.display().to_string(),
            line_number: index + 1,
            kind,
            argument: argument.to_string(),
        });
    }
    Ok(queries)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum QueryKind {
    Search,
    Show,
    Refs,
    Callees,
    Impact,
}

impl QueryKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "search" => Some(Self::Search),
            "show" => Some(Self::Show),
            "refs" => Some(Self::Refs),
            "callees" => Some(Self::Callees),
            "impact" => Some(Self::Impact),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Show => "show",
            Self::Refs => "refs",
            Self::Callees => "callees",
            Self::Impact => "impact",
        }
    }

    fn tolerance(self) -> &'static str {
        match self {
            Self::Search => "unordered set of (kind,file,name); v2 string/config extras ignored",
            Self::Show => "source bytes byte-equal",
            Self::Refs => "set of (file,line,kind) at confidence >= same_module",
            Self::Callees => "set of (file,line,target_name)",
            Self::Impact => "depth=3 set of touched symbol qualified names",
        }
    }
}

#[derive(Debug)]
struct CorpusQuery {
    language: String,
    file: String,
    line_number: usize,
    kind: QueryKind,
    argument: String,
}

#[derive(Debug)]
struct Comparison {
    tolerance: String,
    v1_count: usize,
    v2_count: usize,
    ignored_v2_count: usize,
    violations: Vec<Violation>,
}

impl Comparison {
    fn new(tolerance: &str) -> Self {
        Self {
            tolerance: tolerance.to_string(),
            v1_count: 0,
            v2_count: 0,
            ignored_v2_count: 0,
            violations: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
struct DiffReport {
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    workspace: String,
    corpus: CorpusReport,
    summary: Summary,
    results: Vec<QueryReport>,
}

#[derive(Debug, Serialize)]
struct CorpusReport {
    path: String,
    checksum: String,
    entries: usize,
}

#[derive(Debug, Serialize)]
struct Summary {
    total: usize,
    passed: usize,
    failed: usize,
}

#[derive(Debug, Serialize)]
struct QueryReport {
    language: String,
    source: String,
    line: usize,
    query: String,
    selector: String,
    tolerance: String,
    status: QueryStatus,
    v1_count: usize,
    v2_count: usize,
    #[serde(skip_serializing_if = "is_zero")]
    ignored_v2_count: usize,
    violations: Vec<Violation>,
}

impl QueryReport {
    fn from_comparison(query: &CorpusQuery, status: QueryStatus, comparison: Comparison) -> Self {
        Self {
            language: query.language.clone(),
            source: query.file.clone(),
            line: query.line_number,
            query: query.kind.as_str().to_string(),
            selector: query.argument.clone(),
            tolerance: comparison.tolerance,
            status,
            v1_count: comparison.v1_count,
            v2_count: comparison.v2_count,
            ignored_v2_count: comparison.ignored_v2_count,
            violations: comparison.violations,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum QueryStatus {
    Pass,
    Fail,
}

#[derive(Debug, Serialize)]
struct Violation {
    kind: String,
    rows: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct SearchRow {
    kind: String,
    file: Option<String>,
    name: String,
}

impl From<SearchEntry> for SearchRow {
    fn from(entry: SearchEntry) -> Self {
        Self {
            kind: entry.kind,
            file: entry.file,
            name: entry.name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct RefRow {
    file: String,
    line: usize,
    kind: String,
}

impl From<RefEntry> for RefRow {
    fn from(entry: RefEntry) -> Self {
        Self {
            file: entry.file,
            line: entry.line,
            kind: entry.kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct CalleeRow {
    file: String,
    line: usize,
    target_name: String,
}

impl From<CalleeEntry> for CalleeRow {
    fn from(entry: CalleeEntry) -> Self {
        Self {
            file: entry.file,
            line: entry.line,
            target_name: entry.target_name,
        }
    }
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug)]
enum HarnessError {
    Input(String),
    ReadCorpus {
        path: PathBuf,
        source: io::Error,
    },
    CorpusUtf8 {
        path: PathBuf,
        source: std::string::FromUtf8Error,
    },
    CorpusDrift {
        expected: String,
        actual: String,
        corpus_dir: PathBuf,
    },
    ToleranceViolations(usize),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(message) => f.write_str(message),
            Self::ReadCorpus { path, source } => {
                write!(f, "failed to read corpus file {}: {source}", path.display())
            }
            Self::CorpusUtf8 { path, source } => {
                write!(f, "corpus file {} is not UTF-8: {source}", path.display())
            }
            Self::CorpusDrift {
                expected,
                actual,
                corpus_dir,
            } => write!(
                f,
                "graph-equiv corpus checksum drifted for {}: expected {expected}, got {actual}; update the frozen corpus and EXPECTED_CORPUS_SHA256 together after review",
                corpus_dir.display()
            ),
            Self::ToleranceViolations(count) => {
                write!(
                    f,
                    "graph equivalence found {count} out-of-tolerance diff(s)"
                )
            }
        }
    }
}

impl Error for HarnessError {}
