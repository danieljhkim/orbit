//! Selector parsing and filesystem-anchor helpers for graph queries.

use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Serialize;
use thiserror::Error;

/// Error returned when a selector or legacy path-like scope cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("invalid selector `{input}`: {reason}")]
pub struct SelectorParseError {
    /// The original selector input.
    pub input: String,
    /// Human-readable parse failure reason.
    pub reason: String,
}

/// Canonical graph selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// Directory selector anchored at a workspace-relative or absolute path.
    Dir {
        /// Directory anchor path.
        path: String,
    },
    /// File selector anchored at a workspace-relative or absolute path.
    File {
        /// File anchor path.
        path: String,
    },
    /// Symbol selector anchored at a source path and symbol identity.
    Symbol {
        /// File anchor path.
        path: String,
        /// Opaque symbol name or qualified symbol path.
        symbol: String,
        /// Symbol kind, such as `function`, `method`, or `trait`.
        kind: String,
    },
}

impl Selector {
    /// Parse a list of selector strings.
    pub fn parse_many(raw_selectors: &[String]) -> Result<Vec<Self>, SelectorParseError> {
        raw_selectors
            .iter()
            .map(|selector| selector.parse())
            .collect()
    }

    /// Return the filesystem anchor path for this selector.
    pub fn path(&self) -> &str {
        match self {
            Self::Dir { path } | Self::File { path } | Self::Symbol { path, .. } => path,
        }
    }

    fn with_path(&self, path: String) -> Self {
        match self {
            Self::Dir { .. } => Self::Dir { path },
            Self::File { .. } => Self::File { path },
            Self::Symbol { symbol, kind, .. } => Self::Symbol {
                path,
                symbol: symbol.clone(),
                kind: kind.clone(),
            },
        }
    }

    fn kind(&self) -> ParsedScopeKind {
        match self {
            Self::Dir { .. } => ParsedScopeKind::Dir,
            Self::File { .. } => ParsedScopeKind::File,
            Self::Symbol { .. } => ParsedScopeKind::Symbol,
        }
    }

    /// Return the lookup key used by graph selector indexes.
    pub fn lookup_key(&self) -> SelectorLookupKey {
        match self {
            Self::Dir { path } => SelectorLookupKey::Dir(path.clone()),
            Self::File { path } => SelectorLookupKey::File(path.clone()),
            Self::Symbol { path, symbol, kind } => {
                SelectorLookupKey::Symbol(format!("{path}#{symbol}"), kind.clone())
            }
        }
    }
}

impl Display for Selector {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dir { path } => write!(f, "dir:{path}"),
            Self::File { path } => write!(f, "file:{path}"),
            Self::Symbol { path, symbol, kind } => write!(f, "symbol:{path}#{symbol}:{kind}"),
        }
    }
}

impl FromStr for Selector {
    type Err = SelectorParseError;

    fn from_str(selector: &str) -> Result<Self, Self::Err> {
        let trimmed = selector.trim();
        if let Some(path) = trimmed.strip_prefix("dir:") {
            return Ok(Self::Dir {
                path: normalize_selector_path(selector, path)?,
            });
        }

        if let Some(path) = trimmed.strip_prefix("file:") {
            return Ok(Self::File {
                path: normalize_selector_path(selector, path)?,
            });
        }

        if let Some(remainder) = trimmed.strip_prefix("symbol:") {
            let (location, kind) =
                remainder
                    .rsplit_once(':')
                    .ok_or_else(|| SelectorParseError {
                        input: selector.to_string(),
                        reason: "symbol selectors must use `symbol:<path>#<symbol>:<kind>`"
                            .to_string(),
                    })?;
            if location.is_empty() || kind.is_empty() {
                return Err(SelectorParseError {
                    input: selector.to_string(),
                    reason: "symbol selectors must include both a location and kind".to_string(),
                });
            }
            let (path, symbol) = location.split_once('#').ok_or_else(|| SelectorParseError {
                input: selector.to_string(),
                reason: "symbol selectors must include `#<symbol>`".to_string(),
            })?;
            let path = normalize_selector_path(selector, path)?;
            let symbol = symbol.trim();
            let kind = kind.trim();
            if path.is_empty() || symbol.is_empty() || kind.is_empty() {
                return Err(SelectorParseError {
                    input: selector.to_string(),
                    reason: "symbol selectors must include non-empty path, symbol, and kind"
                        .to_string(),
                });
            }
            return Ok(Self::Symbol {
                path,
                symbol: symbol.to_string(),
                kind: kind.to_string(),
            });
        }

        Err(SelectorParseError {
            input: selector.to_string(),
            reason: "selectors must start with `dir:`, `file:`, or `symbol:`".to_string(),
        })
    }
}

/// Normalized graph selector index key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SelectorLookupKey {
    /// Directory key.
    Dir(String),
    /// File key.
    File(String),
    /// Symbol(location, kind) where location = "path#symbol".
    Symbol(String, String),
}

impl SelectorLookupKey {
    /// Render this lookup key as a canonical selector string.
    pub fn to_selector_string(&self) -> String {
        match self {
            Self::Dir(path) => format!("dir:{path}"),
            Self::File(path) => format!("file:{path}"),
            Self::Symbol(location, kind) => format!("symbol:{location}:{kind}"),
        }
    }
}

/// Convert a selector or legacy path-like input into canonical selector form.
///
/// Accepted inputs are canonical selectors (`file:`, `dir:`, `symbol:`), raw
/// paths, and raw `path:line` / `path:start-end` references. Legacy path
/// references canonicalize to `file:<path>` unless they end with `/` or point
/// at `.` / `..`, in which case they canonicalize to `dir:<path>`.
pub fn canonical_selector(input: &str) -> Result<String, SelectorParseError> {
    Ok(match ParsedScope::parse(input)? {
        ParsedScope::Selector(selector) => selector.to_string(),
        ParsedScope::LegacyPath { path, is_dir_hint } => {
            if is_dir_hint {
                format!("dir:{path}")
            } else {
                format!("file:{path}")
            }
        }
    })
}

/// Canonicalize a selector or legacy path against a workspace root.
///
/// This is stricter than [`canonical_selector`]: absolute anchors inside the
/// workspace are rewritten to workspace-relative form, and legacy paths that
/// resolve to directories on disk canonicalize to `dir:<path>`.
pub fn canonical_selector_in_workspace(
    input: &str,
    workspace: &Path,
) -> Result<String, SelectorParseError> {
    let parsed = ParsedScope::parse(input)?;
    match parsed {
        ParsedScope::Selector(selector) => {
            let path = normalize_workspace_anchor(selector.path(), workspace)?;
            Ok(selector.with_path(path).to_string())
        }
        ParsedScope::LegacyPath { path, is_dir_hint } => {
            let path = normalize_workspace_anchor(path.as_str(), workspace)?;
            let resolved = resolve_workspace_path(workspace, Path::new(&path));
            if is_dir_hint || resolved.is_dir() {
                Ok(format!("dir:{path}"))
            } else {
                Ok(format!("file:{path}"))
            }
        }
    }
}

/// Return the filesystem anchor path for a selector or legacy path-like input.
///
/// For `symbol:` selectors this strips the symbol metadata and returns only the
/// backing file path. Legacy `path:line` references are reduced to their file
/// path anchor.
pub fn anchor_path(selector: &str) -> Result<PathBuf, SelectorParseError> {
    Ok(PathBuf::from(ParsedScope::parse(selector)?.anchor_path()))
}

/// Return whether a selector's filesystem anchor exists in the given workspace.
///
/// Relative anchors are resolved against `workspace`; absolute anchors are
/// checked as-is. Invalid selector strings return `false`.
pub fn exists_in_workspace(selector: &str, workspace: &Path) -> bool {
    let Ok(anchor) = anchor_path(selector) else {
        return false;
    };
    resolve_workspace_path(workspace, anchor.as_path()).exists()
}

/// Return whether two selector/path scopes overlap on the same filesystem
/// anchor or on an ancestor/descendant boundary.
///
/// `symbol:file.rs#one:function` overlaps `symbol:file.rs#two:function` and
/// `file:file.rs`. `dir:src` overlaps any selector anchored under `src/`.
/// Legacy raw paths are treated conservatively and may overlap descendants.
pub fn overlaps(a: &str, b: &str) -> bool {
    let Ok(left) = ParsedScope::parse(a) else {
        return false;
    };
    let Ok(right) = ParsedScope::parse(b) else {
        return false;
    };

    let left_anchor = left.anchor_path();
    let right_anchor = right.anchor_path();
    if left_anchor == right_anchor {
        return true;
    }

    (is_path_ancestor(left_anchor, right_anchor) && left.can_contain_descendants())
        || (is_path_ancestor(right_anchor, left_anchor) && right.can_contain_descendants())
}

/// Return the number of shared path segments between two selector anchors.
pub fn shared_anchor_prefix_depth(left: &str, right: &str) -> usize {
    let Ok(left) = anchor_path(left) else {
        return 0;
    };
    let Ok(right) = anchor_path(right) else {
        return 0;
    };
    let left = normalize_path_text(&left.to_string_lossy()).ok();
    let right = normalize_path_text(&right.to_string_lossy()).ok();
    let (Some(left), Some(right)) = (left, right) else {
        return 0;
    };

    let mut depth = 0usize;
    for (left_part, right_part) in left
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .zip(
            right
                .split('/')
                .filter(|part| !part.is_empty() && *part != "."),
        )
    {
        if left_part != right_part {
            break;
        }
        depth += 1;
    }
    depth
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedScopeKind {
    Dir,
    File,
    Symbol,
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedScope {
    Selector(Selector),
    LegacyPath { path: String, is_dir_hint: bool },
}

impl ParsedScope {
    fn parse(input: &str) -> Result<Self, SelectorParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(SelectorParseError {
                input: input.to_string(),
                reason: "selector input must not be empty".to_string(),
            });
        }

        if trimmed.starts_with("dir:")
            || trimmed.starts_with("file:")
            || trimmed.starts_with("symbol:")
        {
            return Ok(Self::Selector(trimmed.parse()?));
        }

        let (path_like, _had_position_suffix) = strip_position_suffix(trimmed);
        let path = normalize_path_text(path_like).map_err(|reason| SelectorParseError {
            input: input.to_string(),
            reason,
        })?;
        let is_dir_hint = trimmed.ends_with('/') || matches!(path.as_str(), "." | "..");
        Ok(Self::LegacyPath { path, is_dir_hint })
    }

    fn anchor_path(&self) -> &str {
        match self {
            Self::Selector(selector) => selector.path(),
            Self::LegacyPath { path, .. } => path,
        }
    }

    fn kind(&self) -> ParsedScopeKind {
        match self {
            Self::Selector(selector) => selector.kind(),
            Self::LegacyPath { .. } => ParsedScopeKind::Legacy,
        }
    }

    fn can_contain_descendants(&self) -> bool {
        matches!(self.kind(), ParsedScopeKind::Dir | ParsedScopeKind::Legacy)
    }
}

fn normalize_selector_path(
    original_input: &str,
    raw_path: &str,
) -> Result<String, SelectorParseError> {
    normalize_path_text(raw_path).map_err(|reason| SelectorParseError {
        input: original_input.to_string(),
        reason,
    })
}

fn normalize_workspace_anchor(path: &str, workspace: &Path) -> Result<String, SelectorParseError> {
    let normalized = normalize_path_text(path).map_err(|reason| SelectorParseError {
        input: path.to_string(),
        reason,
    })?;
    let resolved = PathBuf::from(&normalized);
    if resolved.is_absolute() {
        let stripped = resolved
            .strip_prefix(workspace)
            .ok()
            .map(|path| path.to_string_lossy().replace('\\', "/"));
        return Ok(stripped.unwrap_or(normalized));
    }
    Ok(normalized)
}

fn normalize_path_text(raw: &str) -> Result<String, String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err("selector path must not be empty".to_string());
    }

    let is_absolute = normalized.starts_with('/');
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if let Some(last) = parts.last()
                    && *last != ".."
                {
                    parts.pop();
                } else if !is_absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }

    if is_absolute {
        return Ok(if parts.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", parts.join("/"))
        });
    }

    Ok(if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    })
}

fn resolve_workspace_path(workspace: &Path, anchor: &Path) -> PathBuf {
    if anchor.is_absolute() {
        anchor.to_path_buf()
    } else {
        workspace.join(anchor)
    }
}

fn strip_position_suffix(input: &str) -> (&str, bool) {
    let mut candidate = input;
    let mut stripped = false;

    loop {
        let Some((base, suffix)) = candidate.rsplit_once(':') else {
            return (candidate, stripped);
        };
        if is_position_segment(suffix) {
            candidate = base;
            stripped = true;
            continue;
        }
        return (candidate, stripped);
    }
}

fn is_position_segment(segment: &str) -> bool {
    is_numeric(segment)
        || segment
            .split_once('-')
            .is_some_and(|(start, end)| is_numeric(start) && is_numeric(end))
}

fn is_numeric(input: &str) -> bool {
    !input.is_empty() && input.chars().all(|ch| ch.is_ascii_digit())
}

fn is_path_ancestor(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with('/'))
}
