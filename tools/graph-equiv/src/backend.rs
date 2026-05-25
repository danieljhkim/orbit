use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use orbit_knowledge::extract::{Language, extract_file};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) type BackendResult<T> = Result<T, BackendError>;
pub(crate) type SearchOutput = Vec<SearchEntry>;
pub(crate) type ShowOutput = Option<Vec<u8>>;
pub(crate) type RefsOutput = Vec<RefEntry>;
pub(crate) type CalleesOutput = Vec<CalleeEntry>;
pub(crate) type ImpactOutput = Vec<String>;
pub(crate) type TraceOutput = Vec<TracePath>;

pub(crate) trait Backend {
    fn sync(&self) -> BackendResult<()> {
        Ok(())
    }

    fn search(&self, query: &str) -> BackendResult<SearchOutput>;
    fn show(&self, selector: &str) -> BackendResult<ShowOutput>;
    fn refs(&self, selector: &str) -> BackendResult<RefsOutput>;
    fn callees(&self, selector: &str) -> BackendResult<CalleesOutput>;
    fn impact(&self, selector: &str, depth: u8) -> BackendResult<ImpactOutput>;
    fn trace(&self, command: &str, depth: u8) -> BackendResult<TraceOutput>;
}

#[derive(Debug, Clone)]
pub(crate) struct V1Backend {
    symbols: Vec<IndexedSymbol>,
    files: Vec<IndexedFile>,
    commands: Vec<IndexedCommand>,
    limit: usize,
}

impl V1Backend {
    pub(crate) fn for_workspace(
        workspace_root: PathBuf,
        _knowledge_dir: Option<PathBuf>,
    ) -> BackendResult<Self> {
        // L-0054: Keep v1 parity checks fixture-scoped so CI does not refresh the full legacy graph.
        let (files, symbols, commands) = load_fixture_index(workspace_root.as_path())?;
        Ok(Self {
            symbols,
            files,
            commands,
            limit: 200,
        })
    }
}

impl Backend for V1Backend {
    fn search(&self, query: &str) -> BackendResult<SearchOutput> {
        let query = query.to_ascii_lowercase();
        Ok(self
            .symbols
            .iter()
            .filter(|symbol| {
                symbol.name.to_ascii_lowercase().contains(query.as_str())
                    || symbol
                        .qualified
                        .to_ascii_lowercase()
                        .contains(query.as_str())
                    || symbol.file.to_ascii_lowercase().contains(query.as_str())
            })
            .take(self.limit)
            .cloned()
            .map(|symbol| SearchEntry {
                selector: symbol.selector,
                kind: "symbol".to_string(),
                file: Some(symbol.file),
                name: symbol.name,
            })
            .collect())
    }

    fn show(&self, selector: &str) -> BackendResult<ShowOutput> {
        if let Some(file) = selector.strip_prefix("file:") {
            return Ok(self
                .files
                .iter()
                .find(|entry| entry.path == file)
                .map(|entry| entry.source.as_bytes().to_vec()));
        }
        if let Some(command) = selector.strip_prefix("command:") {
            return Ok(self
                .commands
                .iter()
                .find(|entry| entry.name == command)
                .and_then(|entry| self.symbol_by_name(entry.handler_symbol.as_str()))
                .map(|entry| entry.source.as_bytes().to_vec()));
        }

        let Some((file, symbol, kind)) = parse_symbol_selector(selector) else {
            return Ok(None);
        };
        Ok(self
            .symbols
            .iter()
            .find(|entry| {
                entry.file == file
                    && entry.kind == kind
                    && (entry.name == symbol || entry.qualified == symbol)
            })
            .map(|entry| entry.source.as_bytes().to_vec()))
    }

    fn refs(&self, selector: &str) -> BackendResult<RefsOutput> {
        let terms = selector_symbol_terms(selector);
        Ok(self
            .symbols
            .iter()
            .filter(|symbol| symbol.selector != selector)
            .filter_map(|symbol| {
                first_term_line(symbol.source.as_str(), &terms).map(|line| RefEntry {
                    file: symbol.file.clone(),
                    line: symbol.start_line.saturating_add(line.saturating_sub(1)),
                    kind: "call".to_string(),
                    confidence: None,
                })
            })
            .take(self.limit)
            .collect())
    }

    fn callees(&self, selector: &str) -> BackendResult<CalleesOutput> {
        let Some((file, symbol, kind)) = parse_symbol_selector(selector) else {
            return Ok(Vec::new());
        };
        let Some(indexed) = self.symbols.iter().find(|entry| {
            entry.file == file
                && entry.kind == kind
                && (entry.name == symbol || entry.qualified == symbol)
        }) else {
            return Ok(Vec::new());
        };
        Ok(extract_call_sites(
            indexed.file.as_str(),
            indexed.source.as_str(),
            indexed.start_line,
        ))
    }

    fn impact(&self, _selector: &str, _depth: u8) -> BackendResult<ImpactOutput> {
        Ok(Vec::new())
    }

    fn trace(&self, command: &str, depth: u8) -> BackendResult<TraceOutput> {
        let Some(command) = self.commands.iter().find(|entry| entry.name == command) else {
            return Ok(Vec::new());
        };
        let Some(root) = self.symbol_by_name(command.handler_symbol.as_str()) else {
            return Ok(Vec::new());
        };
        let mut paths = Vec::new();
        self.trace_symbol(root, depth, vec![root.name.clone()], &mut paths);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

impl V1Backend {
    fn symbol_by_name(&self, name: &str) -> Option<&IndexedSymbol> {
        self.symbols.iter().find(|symbol| symbol.name == name)
    }

    fn trace_symbol(
        &self,
        symbol: &IndexedSymbol,
        depth: u8,
        path: Vec<String>,
        out: &mut Vec<TracePath>,
    ) {
        out.push(TracePath {
            names: path.clone(),
        });
        if depth == 0 {
            return;
        }
        for callee in extract_call_sites(
            symbol.file.as_str(),
            symbol.source.as_str(),
            symbol.start_line,
        ) {
            let Some(next_symbol) = self.symbol_by_name(callee.target_name.as_str()) else {
                continue;
            };
            let mut next_path = path.clone();
            next_path.push(next_symbol.name.clone());
            self.trace_symbol(next_symbol, depth.saturating_sub(1), next_path, out);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct V2Backend {
    workspace_root: PathBuf,
    command: PathBuf,
}

impl V2Backend {
    pub(crate) fn for_workspace(
        workspace_root: PathBuf,
        command: Option<PathBuf>,
    ) -> BackendResult<Self> {
        Ok(Self {
            workspace_root,
            command: command.unwrap_or(resolve_graph_cli_command()?),
        })
    }

    fn run_cli(&self, args: &[&str]) -> BackendResult<Value> {
        let output = Command::new(&self.command)
            .current_dir(&self.workspace_root)
            .args(args)
            .output()
            .map_err(|source| BackendError::Process {
                command: self.command.display().to_string(),
                source,
            })?;

        if !output.status.success() {
            return Err(BackendError::Cli {
                command: format!("{} {}", self.command.display(), args.join(" ")),
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        serde_json::from_slice(&output.stdout).map_err(BackendError::Json)
    }
}

impl Backend for V2Backend {
    fn sync(&self) -> BackendResult<()> {
        let _ = self.run_cli(&["sync"])?;
        Ok(())
    }

    fn search(&self, query: &str) -> BackendResult<SearchOutput> {
        let value = self.run_cli(&["search", query, "--limit", "200"])?;
        let output: V2SearchResult = serde_json::from_value(value).map_err(BackendError::Json)?;
        Ok(output
            .matches
            .into_iter()
            .map(|hit| match hit {
                V2SearchMatch::Symbol { name, path, .. } => SearchEntry {
                    selector: String::new(),
                    kind: "symbol".to_string(),
                    file: Some(path),
                    name,
                },
                V2SearchMatch::StringLiteral { value, path, .. } => SearchEntry {
                    selector: String::new(),
                    kind: "string".to_string(),
                    file: Some(path),
                    name: value,
                },
                V2SearchMatch::Config { value, path, .. } => SearchEntry {
                    selector: String::new(),
                    kind: "config".to_string(),
                    file: Some(path),
                    name: value,
                },
            })
            .collect())
    }

    fn show(&self, selector: &str) -> BackendResult<ShowOutput> {
        let value = self.run_cli(&["show", selector])?;
        if value.is_null() {
            return Ok(None);
        }
        let output: V2ShowResult = serde_json::from_value(value).map_err(BackendError::Json)?;
        Ok(Some(output.bytes))
    }

    fn refs(&self, selector: &str) -> BackendResult<RefsOutput> {
        let value = self.run_cli(&["refs", selector, "--confidence", "same_module"])?;
        let output: V2RefsResult = serde_json::from_value(value).map_err(BackendError::Json)?;
        let refs = output
            .refs
            .into_iter()
            .map(|entry| RefEntry {
                file: entry.file,
                line: entry.line,
                kind: entry.kind,
                confidence: Some(entry.confidence),
            })
            .chain(output.relations.into_iter().map(|entry| RefEntry {
                file: entry.file,
                line: entry.line,
                kind: entry.kind,
                confidence: Some(entry.confidence),
            }))
            .collect();
        Ok(refs)
    }

    fn callees(&self, selector: &str) -> BackendResult<CalleesOutput> {
        let file = selector_file(selector).ok_or_else(|| {
            BackendError::InvalidData(format!(
                "callees requires a file-backed selector: {selector}"
            ))
        })?;
        let value = self.run_cli(&["callees", selector])?;
        let output: V2CalleesResult = serde_json::from_value(value).map_err(BackendError::Json)?;
        let callees = output
            .callees
            .into_iter()
            .map(|entry| CalleeEntry {
                file: file.clone(),
                line: entry.line,
                target_name: entry.target_name,
            })
            .collect::<Vec<_>>();
        Ok(callees)
    }

    fn impact(&self, selector: &str, depth: u8) -> BackendResult<ImpactOutput> {
        let depth = depth.to_string();
        let value = self.run_cli(&["impact", selector, "--depth", depth.as_str()])?;
        let output: V2ImpactResult = serde_json::from_value(value).map_err(BackendError::Json)?;
        Ok(output
            .touched
            .into_iter()
            .map(|entry| entry.qualified_name)
            .collect())
    }

    fn trace(&self, command: &str, depth: u8) -> BackendResult<TraceOutput> {
        let depth = depth.to_string();
        let value = self.run_cli(&["trace", command, "--depth", depth.as_str()])?;
        let output: V2TraceResult = serde_json::from_value(value).map_err(BackendError::Json)?;
        let mut paths = Vec::new();
        if let Some(root) = output.root {
            collect_trace_paths(&root, Vec::new(), &mut paths);
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[derive(Debug, Clone)]
struct IndexedFile {
    path: String,
    source: String,
}

#[derive(Debug, Clone)]
struct IndexedSymbol {
    selector: String,
    file: String,
    name: String,
    qualified: String,
    kind: String,
    source: String,
    start_line: usize,
}

#[derive(Debug, Clone)]
struct IndexedCommand {
    name: String,
    handler_symbol: String,
}

fn load_fixture_index(
    workspace_root: &Path,
) -> BackendResult<(Vec<IndexedFile>, Vec<IndexedSymbol>, Vec<IndexedCommand>)> {
    let fixture_root = workspace_root.join("tools/graph-equiv/fixtures");
    let mut paths = Vec::new();
    collect_files(fixture_root.as_path(), &mut paths)?;
    paths.sort();

    let mut files = Vec::new();
    let mut symbols = Vec::new();
    let mut commands = Vec::new();
    for path in paths {
        let Some(language) = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(Language::from_extension)
        else {
            continue;
        };
        let source =
            fs::read_to_string(path.as_path()).map_err(|source| BackendError::ReadFile {
                path: path.clone(),
                source,
            })?;
        let rel_path = relative_slash_path(workspace_root, path.as_path())?;
        let extracted = extract_file(source.as_str(), language);
        files.push(IndexedFile {
            path: rel_path.clone(),
            source: source.clone(),
        });
        commands.extend(extract_fixture_commands(rel_path.as_str(), source.as_str()));
        symbols.extend(extracted.leaves.into_iter().map(|leaf| IndexedSymbol {
            selector: format!("symbol:{}#{}:{}", rel_path, leaf.qualified_name, leaf.kind),
            file: rel_path.clone(),
            name: leaf.name,
            qualified: leaf.qualified_name,
            kind: leaf.kind,
            source: leaf.source,
            start_line: leaf.start_line,
        }));
    }
    Ok((files, symbols, commands))
}

fn extract_fixture_commands(file: &str, source: &str) -> Vec<IndexedCommand> {
    if file.ends_with(".py") {
        return extract_python_click_commands(source);
    }
    Vec::new()
}

fn extract_python_click_commands(source: &str) -> Vec<IndexedCommand> {
    let mut commands = Vec::new();
    let mut pending_click_command = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("@click.command") {
            pending_click_command = true;
            continue;
        }
        if pending_click_command {
            pending_click_command = false;
            if let Some(function_name) = trimmed
                .strip_prefix("def ")
                .and_then(|rest| rest.split_once('(').map(|(name, _)| name.trim()))
                .filter(|name| !name.is_empty())
            {
                commands.push(IndexedCommand {
                    name: function_name.replace('_', "-"),
                    handler_symbol: function_name.to_string(),
                });
            }
        }
    }
    commands
}

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) -> BackendResult<()> {
    for entry in fs::read_dir(root).map_err(|source| BackendError::ReadFile {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| BackendError::ReadFile {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(path.as_path(), out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn relative_slash_path(root: &Path, path: &Path) -> BackendResult<String> {
    let relative = path.strip_prefix(root).map_err(|source| {
        BackendError::InvalidData(format!(
            "fixture path {} is outside {}: {source}",
            path.display(),
            root.display()
        ))
    })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct SearchEntry {
    pub(crate) selector: String,
    pub(crate) kind: String,
    pub(crate) file: Option<String>,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct RefEntry {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) confidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct CalleeEntry {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) target_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct TracePath {
    pub(crate) names: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct V2SearchResult {
    matches: Vec<V2SearchMatch>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum V2SearchMatch {
    Symbol {
        name: String,
        path: String,
        #[serde(rename = "line")]
        _line: usize,
    },
    #[serde(rename = "string")]
    StringLiteral {
        value: String,
        path: String,
        #[serde(rename = "line")]
        _line: usize,
    },
    Config {
        value: String,
        path: String,
        #[serde(rename = "line")]
        _line: usize,
    },
}

#[derive(Debug, Deserialize)]
struct V2ShowResult {
    bytes: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct V2RefsResult {
    refs: Vec<V2RefEntry>,
    relations: Vec<V2RelationEntry>,
}

#[derive(Debug, Deserialize)]
struct V2RefEntry {
    file: String,
    line: usize,
    kind: String,
    confidence: String,
}

#[derive(Debug, Deserialize)]
struct V2RelationEntry {
    file: String,
    line: usize,
    kind: String,
    confidence: String,
}

#[derive(Debug, Deserialize)]
struct V2CalleesResult {
    callees: Vec<V2CalleeEntry>,
}

#[derive(Debug, Deserialize)]
struct V2CalleeEntry {
    target_name: String,
    line: usize,
}

#[derive(Debug, Deserialize)]
struct V2ImpactResult {
    touched: Vec<V2ImpactEntry>,
}

#[derive(Debug, Deserialize)]
struct V2ImpactEntry {
    qualified_name: String,
}

#[derive(Debug, Deserialize)]
struct V2TraceResult {
    root: Option<V2TraceNode>,
}

#[derive(Debug, Deserialize)]
struct V2TraceNode {
    name: String,
    children: Vec<V2TraceNode>,
}

fn collect_trace_paths(node: &V2TraceNode, mut path: Vec<String>, out: &mut Vec<TracePath>) {
    path.push(node.name.clone());
    out.push(TracePath {
        names: path.clone(),
    });
    for child in &node.children {
        collect_trace_paths(child, path.clone(), out);
    }
}

#[derive(Debug)]
pub(crate) enum BackendError {
    Json(serde_json::Error),
    Process {
        command: String,
        source: io::Error,
    },
    Cli {
        command: String,
        status: Option<i32>,
        stderr: String,
    },
    ReadFile {
        path: PathBuf,
        source: io::Error,
    },
    InvalidData(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "failed to parse orbit-graph-cli JSON: {error}"),
            Self::Process { command, source } => {
                write!(f, "failed to run `{command}`: {source}")
            }
            Self::Cli {
                command,
                status,
                stderr,
            } => write!(
                f,
                "`{command}` failed with status {}: {stderr}",
                status
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            ),
            Self::ReadFile { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::InvalidData(message) => f.write_str(message),
        }
    }
}

impl Error for BackendError {}

fn resolve_graph_cli_command() -> BackendResult<PathBuf> {
    if let Some(value) = std::env::var_os("ORBIT_GRAPH_CLI")
        && !value.is_empty()
    {
        return Ok(PathBuf::from(value));
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let mut candidate = dir.join("orbit-graph-cli");
        if cfg!(windows) {
            candidate.set_extension("exe");
        }
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Ok(PathBuf::from("orbit-graph-cli"))
}

fn parse_symbol_selector(selector: &str) -> Option<(String, String, String)> {
    let rest = selector.strip_prefix("symbol:")?;
    let (without_kind, kind) = rest.rsplit_once(':')?;
    let (file, symbol) = without_kind.split_once('#')?;
    Some((file.to_string(), symbol.to_string(), kind.to_string()))
}

fn selector_symbol_terms(selector: &str) -> Vec<String> {
    let Some((_, symbol, _)) = parse_symbol_selector(selector) else {
        return Vec::new();
    };
    let mut terms = vec![symbol.clone()];
    let simple = simple_symbol_name(symbol.as_str());
    if simple != symbol {
        terms.push(simple);
    }
    terms
}

fn selector_file(selector: &str) -> Option<String> {
    if let Some(rest) = selector.strip_prefix("file:") {
        return Some(rest.to_string());
    }
    parse_symbol_selector(selector).map(|(file, _, _)| file)
}

fn simple_symbol_name(symbol: &str) -> String {
    symbol
        .rsplit("::")
        .next()
        .unwrap_or(symbol)
        .rsplit('.')
        .next()
        .unwrap_or(symbol)
        .to_string()
}

fn first_term_line(source: &str, terms: &[String]) -> Option<usize> {
    terms
        .iter()
        .filter_map(|term| {
            find_identifier(source, term).map(|offset| line_for_byte(source.as_bytes(), offset))
        })
        .min()
}

fn find_identifier(source: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut search_start = 0usize;
    while let Some(relative_match) = source[search_start..].find(needle) {
        let match_start = search_start + relative_match;
        let match_end = match_start + needle.len();
        let before = source[..match_start].chars().next_back();
        let after = source[match_end..].chars().next();
        let before_ok = before.is_none_or(|ch| !is_ident_continue_char(ch));
        let after_ok = after.is_none_or(|ch| !is_ident_continue_char(ch));
        if before_ok && after_ok {
            return Some(match_start);
        }
        search_start = match_end;
    }
    None
}

fn extract_call_sites(file: &str, source: &str, start_line: usize) -> Vec<CalleeEntry> {
    let mut entries = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !is_ident_start(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && is_ident_continue(bytes[index]) {
            index += 1;
        }
        let name = &source[start..index];
        let mut cursor = index;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor < bytes.len()
            && bytes[cursor] == b'('
            && !is_ignored_call_name(name)
            && !is_declaration_name(source, start)
        {
            entries.push(CalleeEntry {
                file: file.to_string(),
                line: start_line.saturating_add(line_for_byte(bytes, start).saturating_sub(1)),
                target_name: name.to_string(),
            });
        }
    }
    entries.sort();
    entries.dedup();
    entries
}

fn is_declaration_name(source: &str, ident_start: usize) -> bool {
    previous_word(source, ident_start)
        .is_some_and(|word| matches!(word.as_str(), "def" | "fn" | "func" | "function"))
}

fn previous_word(source: &str, before: usize) -> Option<String> {
    let prefix = source.get(..before)?;
    let trimmed = prefix.trim_end();
    let end = trimmed.len();
    let start = trimmed
        .char_indices()
        .rev()
        .find(|(_, ch)| !is_ident_continue_char(*ch))
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    (start < end).then(|| trimmed[start..end].to_string())
}

fn is_ignored_call_name(name: &str) -> bool {
    matches!(
        name,
        "if" | "for"
            | "while"
            | "loop"
            | "match"
            | "switch"
            | "catch"
            | "return"
            | "sizeof"
            | "Some"
            | "Ok"
            | "Err"
            | "String"
            | "Promise"
    )
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ident_continue(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn is_ident_continue_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn line_for_byte(source: &[u8], offset: usize) -> usize {
    source
        .get(..offset.min(source.len()))
        .unwrap_or(source)
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}
