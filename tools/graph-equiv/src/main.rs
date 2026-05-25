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
use std::time::{Duration, Instant};

use backend::{
    Backend, CalleeEntry, CalleesOutput, ImpactOutput, RefEntry, RefsOutput, SearchEntry,
    SearchOutput, ShowOutput, TraceOutput, V1Backend, V2Backend,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const EXPECTED_CORPUS_SHA256: &str =
    "3c07c90f4635c6e65a206cface7a94b077018f868deb229303b843ca98be7b05";
const LANGUAGES: [&str; 4] = ["rust", "typescript", "python", "go"];
const IMPACT_DEPTH: u8 = 3;
const TRACE_DEPTH: u8 = 3;

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
    let performance = PerformanceSummary::from_results(&results);
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
            categorized_diffs: 0,
            uncategorized_diffs: failed,
        },
        performance,
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
        QueryKind::Sync => {
            let v1_rows = measure(|| v1.sync());
            let v2_rows = measure(|| v2.sync());
            compare_backend_outputs(query, v1_rows, v2_rows, compare_sync)
        }
        QueryKind::Search => {
            let v1_rows = measure(|| v1.search(query.argument.as_str()));
            let v2_rows = measure(|| v2.search(query.argument.as_str()));
            compare_backend_outputs(query, v1_rows, v2_rows, compare_search)
        }
        QueryKind::Show => {
            let v1_rows = measure(|| v1.show(query.argument.as_str()));
            let v2_rows = measure(|| v2.show(query.argument.as_str()));
            compare_backend_outputs(query, v1_rows, v2_rows, compare_show)
        }
        QueryKind::Refs => {
            let v1_rows = measure(|| v1.refs(query.argument.as_str()));
            let v2_rows = measure(|| v2.refs(query.argument.as_str()));
            compare_backend_outputs(query, v1_rows, v2_rows, compare_refs)
        }
        QueryKind::Callees => {
            let v1_rows = measure(|| v1.callees(query.argument.as_str()));
            let v2_rows = measure(|| v2.callees(query.argument.as_str()));
            compare_backend_outputs(query, v1_rows, v2_rows, compare_callees)
        }
        QueryKind::Impact => {
            let v1_rows = measure(|| v1.impact(query.argument.as_str(), IMPACT_DEPTH));
            let v2_rows = measure(|| v2.impact(query.argument.as_str(), IMPACT_DEPTH));
            compare_backend_outputs(query, v1_rows, v2_rows, compare_impact)
        }
        QueryKind::Trace => {
            let v1_rows = measure(|| v1.trace(query.argument.as_str(), TRACE_DEPTH));
            let v2_rows = measure(|| v2.trace(query.argument.as_str(), TRACE_DEPTH));
            compare_backend_outputs(query, v1_rows, v2_rows, compare_trace)
        }
    }
}

fn measure<T>(
    run: impl FnOnce() -> Result<T, backend::BackendError>,
) -> Measured<Result<T, backend::BackendError>> {
    let start = Instant::now();
    let output = run();
    Measured {
        output,
        duration: start.elapsed(),
    }
}

fn compare_backend_outputs<T, F>(
    query: &CorpusQuery,
    v1: Measured<Result<T, backend::BackendError>>,
    v2: Measured<Result<T, backend::BackendError>>,
    compare: F,
) -> QueryReport
where
    F: FnOnce(T, T) -> Comparison,
{
    let timing = TimingReport::from_durations(v1.duration, v2.duration);
    match (v1.output, v2.output) {
        (Ok(v1_output), Ok(v2_output)) => {
            let comparison = compare(v1_output, v2_output);
            let status = if comparison.violations.is_empty() {
                QueryStatus::Pass
            } else {
                QueryStatus::Fail
            };
            QueryReport::from_comparison(query, status, comparison, timing)
        }
        (v1_result, v2_result) => QueryReport::from_comparison(
            query,
            QueryStatus::Fail,
            Comparison {
                tolerance: query.kind.tolerance().to_string(),
                v1_count: usize::from(v1_result.is_ok()),
                v2_count: usize::from(v2_result.is_ok()),
                ignored_v2_count: 0,
                violations: backend_errors(v1_result.err(), v2_result.err()),
            },
            timing,
        ),
    }
}

fn backend_errors(
    v1: Option<backend::BackendError>,
    v2: Option<backend::BackendError>,
) -> Vec<Violation> {
    let mut rows = Vec::new();
    if let Some(error) = v1 {
        rows.push(json!({ "backend": "v1", "error": error.to_string() }));
    }
    if let Some(error) = v2 {
        rows.push(json!({ "backend": "v2", "error": error.to_string() }));
    }
    vec![Violation {
        kind: "backend_error".to_string(),
        rows: json!(rows),
    }]
}

fn compare_sync(_: (), _: ()) -> Comparison {
    let mut comparison = Comparison::new("sync: both backends complete successfully");
    comparison.v1_count = 1;
    comparison.v2_count = 1;
    comparison
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

fn compare_trace(v1: TraceOutput, v2: TraceOutput) -> Comparison {
    let v1_rows = v1.into_iter().collect::<BTreeSet<_>>();
    let v2_rows = v2.into_iter().collect::<BTreeSet<_>>();
    set_comparison(
        "trace: set of root-to-callee name paths",
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
    Sync,
    Search,
    Show,
    Refs,
    Callees,
    Impact,
    Trace,
}

impl QueryKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "sync" => Some(Self::Sync),
            "search" => Some(Self::Search),
            "show" => Some(Self::Show),
            "refs" => Some(Self::Refs),
            "callees" => Some(Self::Callees),
            "impact" => Some(Self::Impact),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Search => "search",
            Self::Show => "show",
            Self::Refs => "refs",
            Self::Callees => "callees",
            Self::Impact => "impact",
            Self::Trace => "trace",
        }
    }

    fn tolerance(self) -> &'static str {
        match self {
            Self::Sync => "both backends complete successfully",
            Self::Search => "unordered set of (kind,file,name); v2 string/config extras ignored",
            Self::Show => "source bytes byte-equal",
            Self::Refs => "set of (file,line,kind) at confidence >= same_module",
            Self::Callees => "set of (file,line,target_name)",
            Self::Impact => "depth=3 set of touched symbol qualified names",
            Self::Trace => "set of root-to-callee name paths",
        }
    }
}

struct Measured<T> {
    output: T,
    duration: Duration,
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
    performance: PerformanceSummary,
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
    categorized_diffs: usize,
    uncategorized_diffs: usize,
}

#[derive(Debug, Serialize)]
struct PerformanceSummary {
    v1_median_us: u128,
    v1_p95_us: u128,
    v2_median_us: u128,
    v2_p95_us: u128,
}

impl PerformanceSummary {
    fn from_results(results: &[QueryReport]) -> Self {
        let mut v1 = results
            .iter()
            .map(|result| result.timing.v1_us)
            .collect::<Vec<_>>();
        let mut v2 = results
            .iter()
            .map(|result| result.timing.v2_us)
            .collect::<Vec<_>>();
        Self {
            v1_median_us: percentile(&mut v1, 50),
            v1_p95_us: percentile(&mut v1, 95),
            v2_median_us: percentile(&mut v2, 50),
            v2_p95_us: percentile(&mut v2, 95),
        }
    }
}

fn percentile(values: &mut [u128], pct: usize) -> u128 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let index = ((values.len() * pct).div_ceil(100)).saturating_sub(1);
    values[index.min(values.len() - 1)]
}

#[derive(Debug, Serialize)]
struct TimingReport {
    v1_us: u128,
    v2_us: u128,
}

impl TimingReport {
    fn from_durations(v1: Duration, v2: Duration) -> Self {
        Self {
            v1_us: v1.as_micros(),
            v2_us: v2.as_micros(),
        }
    }
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
    timing: TimingReport,
    violations: Vec<Violation>,
}

impl QueryReport {
    fn from_comparison(
        query: &CorpusQuery,
        status: QueryStatus,
        comparison: Comparison,
        timing: TimingReport,
    ) -> Self {
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
            timing,
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
