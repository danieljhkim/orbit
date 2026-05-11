//! Reference sweep across `docs/design/**/*.md` (excluding `4_decisions.md`).
//!
//! Rewrites four reference forms when the resolution is unambiguous, and logs
//! the rest to the migration report. See `2_design.md` §7.4 for the four
//! forms and the ambiguity ceiling. The sweeper refuses to guess.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

use super::{RewriteRecord, UnresolvedRefRecord};

/// `(feature, legacy_id) → global_id` map produced by `ingest`.
pub(super) type IdMap = HashMap<(String, String), String>;

#[derive(Debug, Default)]
pub(super) struct SweepOutcome {
    pub rewrites: Vec<RewriteRecord>,
    pub unresolved: Vec<UnresolvedRefRecord>,
}

pub(super) fn run_sweep(
    design_root: &Path,
    id_map: &IdMap,
    dry_run: bool,
) -> Result<SweepOutcome, OrbitError> {
    let mut outcome = SweepOutcome::default();
    if !design_root.is_dir() {
        return Ok(outcome);
    }

    // Precompute the set of features that have ADR-NNN entries — used to
    // detect ambiguous cross-feature bare references.
    let mut features_by_legacy: HashMap<String, Vec<String>> = HashMap::new();
    for (feature, legacy_id) in id_map.keys() {
        features_by_legacy
            .entry(legacy_id.clone())
            .or_default()
            .push(feature.clone());
    }

    let files = collect_design_markdown(design_root)?;
    for file in files {
        let feature = feature_for_path(design_root, &file);
        let raw = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(err) => {
                return Err(OrbitError::Io(format!("read {}: {err}", file.display())));
            }
        };
        let (rewritten, file_rewrites, file_unresolved) =
            sweep_file(&raw, &file, feature.as_deref(), id_map, &features_by_legacy);
        outcome.rewrites.extend(file_rewrites);
        outcome.unresolved.extend(file_unresolved);
        if !dry_run && rewritten != raw {
            fs::write(&file, &rewritten)
                .map_err(|e| OrbitError::Io(format!("write {}: {e}", file.display())))?;
        }
    }

    Ok(outcome)
}

fn collect_design_markdown(design_root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut out = Vec::new();
    visit_dir(design_root, &mut out)?;
    out.sort();
    Ok(out)
}

fn visit_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), OrbitError> {
    for entry in
        fs::read_dir(dir).map_err(|e| OrbitError::Io(format!("read {}: {e}", dir.display())))?
    {
        let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .map_err(|e| OrbitError::Io(e.to_string()))?;
        if ft.is_dir() {
            visit_dir(&path, out)?;
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("4_decisions.md") {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
    Ok(())
}

fn feature_for_path(design_root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(design_root).ok()?;
    rel.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .map(|s| s.to_string())
}

/// Sweep a single file. Returns `(new_contents, rewrites, unresolved)`.
///
/// Lines inside fenced code blocks (` ``` ` toggle) are passed through
/// untouched — they're literal example content, not references.
fn sweep_file(
    raw: &str,
    path: &Path,
    feature: Option<&str>,
    id_map: &IdMap,
    features_by_legacy: &HashMap<String, Vec<String>>,
) -> (String, Vec<RewriteRecord>, Vec<UnresolvedRefRecord>) {
    let mut out = String::with_capacity(raw.len());
    let mut rewrites = Vec::new();
    let mut unresolved = Vec::new();

    let mut in_fence = false;
    for (idx, line) in raw.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let line_no = idx + 1;
        let (rewritten, line_rewrites, line_unresolved) =
            sweep_line(line, path, line_no, feature, id_map, features_by_legacy);
        out.push_str(&rewritten);
        out.push('\n');
        rewrites.extend(line_rewrites);
        unresolved.extend(line_unresolved);
    }
    // Preserve the trailing newline if the original had one.
    if !raw.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    (out, rewrites, unresolved)
}

fn sweep_line(
    line: &str,
    path: &Path,
    line_no: usize,
    feature: Option<&str>,
    id_map: &IdMap,
    features_by_legacy: &HashMap<String, Vec<String>>,
) -> (String, Vec<RewriteRecord>, Vec<UnresolvedRefRecord>) {
    let mut rewrites = Vec::new();
    let mut unresolved = Vec::new();
    let mut result = String::with_capacity(line.len());

    let bytes = line.as_bytes();
    let mut i = 0;
    // Track inline-code spans (single-backtick toggle) so literal example
    // strings like `"activity-job/ADR-017"` aren't rewritten.
    let mut in_inline_code = false;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            in_inline_code = !in_inline_code;
            result.push('`');
            i += 1;
            continue;
        }
        if in_inline_code {
            let ch = line[i..].chars().next().unwrap_or('\0');
            result.push(ch);
            i += ch.len_utf8();
            continue;
        }
        // Form 4: explicit cross-feature `<feature>/ADR-NNN`. Recognized when
        // an alphanumeric/dash run is followed by `/ADR-` and digits, NOT
        // inside an existing bracket (we leave bracketed refs to form 1).
        if let Some(consumed) = try_cross_feature(
            line,
            i,
            path,
            line_no,
            id_map,
            &mut result,
            &mut rewrites,
            &mut unresolved,
        ) {
            i += consumed;
            continue;
        }

        // Form 1: bracketed local `[ADR-NNN]`.
        if bytes[i] == b'['
            && let Some(end) = find_closing_bracket(bytes, i)
        {
            let inner = &line[i + 1..end];
            if let Some(legacy_id) = parse_local_adr_token(inner) {
                let (replacement, rewrite, unresolved_entry) = resolve_local_adr_token(
                    feature,
                    legacy_id,
                    path,
                    line_no,
                    &line[i..=end],
                    id_map,
                    features_by_legacy,
                );
                result.push_str(&replacement);
                if let Some(r) = rewrite {
                    rewrites.push(r);
                }
                if let Some(u) = unresolved_entry {
                    unresolved.push(u);
                }
                i = end + 1;
                continue;
            }
        }

        // Form 2: prose mentions `see ADR-NNN` / `per ADR-NNN` / `via ADR-NNN`.
        if let Some(consumed) = try_prose_mention(
            line,
            i,
            path,
            line_no,
            feature,
            id_map,
            features_by_legacy,
            &mut result,
            &mut rewrites,
            &mut unresolved,
        ) {
            i += consumed;
            continue;
        }

        let ch = line[i..].chars().next().unwrap_or('\0');
        result.push(ch);
        i += ch.len_utf8();
    }

    (result, rewrites, unresolved)
}

fn find_closing_bracket(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b']' {
            return Some(i);
        }
        if bytes[i] == b'\n' || bytes[i] == b'[' {
            return None;
        }
        i += 1;
    }
    None
}

fn parse_local_adr_token(inner: &str) -> Option<&str> {
    // Accept `ADR-NNN` where NNN is 1+ digits and the token has no other text.
    if !inner.starts_with("ADR-") {
        return None;
    }
    let digits = &inner[4..];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // Already a 4-digit (global) ID? Don't rewrite — leave it alone.
    if digits.len() >= 4 && digits.chars().next().is_some_and(|c| c == '0') {
        return None;
    }
    Some(inner)
}

fn resolve_local_adr_token(
    feature: Option<&str>,
    legacy_id: &str,
    path: &Path,
    line_no: usize,
    original_with_brackets: &str,
    id_map: &IdMap,
    features_by_legacy: &HashMap<String, Vec<String>>,
) -> (String, Option<RewriteRecord>, Option<UnresolvedRefRecord>) {
    let feature_name = match feature {
        Some(name) => name,
        None => {
            return (
                original_with_brackets.to_string(),
                None,
                Some(UnresolvedRefRecord {
                    file: path.to_path_buf(),
                    line: line_no,
                    original: original_with_brackets.to_string(),
                    reason: "file is outside docs/design/<feature>/; cannot infer feature scope"
                        .to_string(),
                }),
            );
        }
    };

    if let Some(global) = id_map.get(&(feature_name.to_string(), legacy_id.to_string())) {
        let replacement = format!("[{global}]");
        let rewrite = RewriteRecord {
            file: path.to_path_buf(),
            line: line_no,
            original: original_with_brackets.to_string(),
            rewritten: replacement.clone(),
        };
        return (replacement, Some(rewrite), None);
    }

    // Bare cross-feature: more than one feature has this legacy ID and the
    // current feature doesn't.
    let unresolved = match features_by_legacy.get(legacy_id) {
        Some(features) if features.len() > 1 => UnresolvedRefRecord {
            file: path.to_path_buf(),
            line: line_no,
            original: original_with_brackets.to_string(),
            reason: format!(
                "ambiguous: `{legacy_id}` exists in {} features; use explicit `<feature>/ADR-NNN`",
                features.len()
            ),
        },
        _ => UnresolvedRefRecord {
            file: path.to_path_buf(),
            line: line_no,
            original: original_with_brackets.to_string(),
            reason: format!("no global ADR resolves `{feature_name}/{legacy_id}`"),
        },
    };
    (original_with_brackets.to_string(), None, Some(unresolved))
}

#[allow(clippy::too_many_arguments)]
fn try_prose_mention(
    line: &str,
    pos: usize,
    path: &Path,
    line_no: usize,
    feature: Option<&str>,
    id_map: &IdMap,
    features_by_legacy: &HashMap<String, Vec<String>>,
    result: &mut String,
    rewrites: &mut Vec<RewriteRecord>,
    unresolved: &mut Vec<UnresolvedRefRecord>,
) -> Option<usize> {
    let suffix = &line[pos..];
    for prefix in ["see ADR-", "per ADR-", "via ADR-", "See ADR-", "Per ADR-"] {
        if let Some(rest) = suffix.strip_prefix(prefix) {
            // Reject if the preceding char is alphanumeric (we'd be inside a
            // word).
            if pos > 0
                && let Some(prev) = line[..pos].chars().last()
                && prev.is_alphanumeric()
            {
                continue;
            }
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                continue;
            }
            let trailing = &rest[digits.len()..];
            // Stop the match at the next non-digit boundary character.
            let boundary_ok = trailing
                .chars()
                .next()
                .map(|c| !c.is_alphanumeric() && c != '-')
                .unwrap_or(true);
            if !boundary_ok {
                continue;
            }
            let legacy = format!("ADR-{digits}");
            let original = format!("{prefix}{digits}");
            let feature_name = match feature {
                Some(f) => f.to_string(),
                None => {
                    unresolved.push(UnresolvedRefRecord {
                        file: path.to_path_buf(),
                        line: line_no,
                        original: original.clone(),
                        reason:
                            "file is outside docs/design/<feature>/; cannot infer feature scope"
                                .to_string(),
                    });
                    result.push_str(&original);
                    return Some(prefix.len() + digits.len());
                }
            };
            if let Some(global) = id_map.get(&(feature_name.clone(), legacy.clone())) {
                let verb = prefix
                    .trim_end_matches(" ADR-")
                    .trim()
                    .trim_end_matches(['S', 'P', 'V'])
                    .to_string();
                let _ = verb; // suppress unused-var lint; verb stays in the text
                let replacement = format!("{}[{global}]", &prefix[..prefix.len() - 4]);
                rewrites.push(RewriteRecord {
                    file: path.to_path_buf(),
                    line: line_no,
                    original: original.clone(),
                    rewritten: replacement.clone(),
                });
                result.push_str(&replacement);
                return Some(prefix.len() + digits.len());
            }
            let reason = match features_by_legacy.get(&legacy) {
                Some(features) if features.len() > 1 => format!(
                    "ambiguous: `{legacy}` exists in {} features; use explicit `<feature>/ADR-NNN`",
                    features.len()
                ),
                _ => format!("no global ADR resolves `{feature_name}/{legacy}`"),
            };
            unresolved.push(UnresolvedRefRecord {
                file: path.to_path_buf(),
                line: line_no,
                original: original.clone(),
                reason,
            });
            result.push_str(&original);
            return Some(prefix.len() + digits.len());
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn try_cross_feature(
    line: &str,
    pos: usize,
    path: &Path,
    line_no: usize,
    id_map: &IdMap,
    result: &mut String,
    rewrites: &mut Vec<RewriteRecord>,
    unresolved: &mut Vec<UnresolvedRefRecord>,
) -> Option<usize> {
    // Pattern: <feature>/ADR-NNN where <feature> is [a-z0-9-]+. We only match
    // when the preceding char is not alphanumeric (so we don't accidentally
    // sweep paths like docs/design/<feature>/4_decisions.md).
    let suffix = &line[pos..];
    let slash = suffix.find("/ADR-")?;
    if slash == 0 {
        return None;
    }
    let feature_part = &suffix[..slash];
    // Validate the feature token.
    if feature_part
        .chars()
        .any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    {
        return None;
    }
    if pos > 0
        && let Some(prev) = line[..pos].chars().last()
        && (prev.is_alphanumeric() || prev == '/')
    {
        return None;
    }
    let after_marker = slash + "/ADR-".len();
    let digits: String = suffix[after_marker..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    let trailing_idx = after_marker + digits.len();
    let trailing = suffix
        .as_bytes()
        .get(trailing_idx)
        .copied()
        .unwrap_or(b'\n');
    let trailing_ok = !(trailing.is_ascii_alphanumeric() || trailing == b'-');
    if !trailing_ok {
        return None;
    }
    let legacy = format!("ADR-{digits}");
    let original = format!("{feature_part}/{legacy}");
    let consumed = slash + "/ADR-".len() + digits.len();

    if let Some(global) = id_map.get(&(feature_part.to_string(), legacy.clone())) {
        let replacement = format!("[{global}]");
        rewrites.push(RewriteRecord {
            file: path.to_path_buf(),
            line: line_no,
            original: original.clone(),
            rewritten: replacement.clone(),
        });
        result.push_str(&replacement);
        return Some(consumed);
    }
    unresolved.push(UnresolvedRefRecord {
        file: path.to_path_buf(),
        line: line_no,
        original: original.clone(),
        reason: format!("no global ADR resolves `{original}`"),
    });
    result.push_str(&original);
    Some(consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_map(entries: &[(&str, &str, &str)]) -> IdMap {
        entries
            .iter()
            .map(|(f, l, g)| ((f.to_string(), l.to_string()), g.to_string()))
            .collect()
    }

    fn features_by_legacy(id_map: &IdMap) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for (feature, legacy) in id_map.keys() {
            out.entry(legacy.clone()).or_default().push(feature.clone());
        }
        out
    }

    #[test]
    fn bracketed_local_ref_rewrites_to_global() {
        let map = id_map(&[("activity-job", "ADR-048", "ADR-0042")]);
        let fbl = features_by_legacy(&map);
        let (out, rewrites, unresolved) = sweep_file(
            "See [ADR-048] in this feature.\n",
            Path::new("docs/design/activity-job/2_design.md"),
            Some("activity-job"),
            &map,
            &fbl,
        );
        assert!(out.contains("[ADR-0042]"), "{out}");
        assert!(!out.contains("[ADR-048]"));
        assert_eq!(rewrites.len(), 1);
        assert!(unresolved.is_empty());
    }

    #[test]
    fn ambiguous_bracketed_ref_is_logged_not_rewritten() {
        let map = id_map(&[
            ("activity-job", "ADR-017", "ADR-0017"),
            ("auditability", "ADR-017", "ADR-0098"),
        ]);
        let fbl = features_by_legacy(&map);
        // File is in a third folder with no ADR-017 of its own.
        let (out, rewrites, unresolved) = sweep_file(
            "See [ADR-017].\n",
            Path::new("docs/design/groundhog/2_design.md"),
            Some("groundhog"),
            &map,
            &fbl,
        );
        assert!(out.contains("[ADR-017]"), "should leave original: {out}");
        assert!(rewrites.is_empty());
        assert_eq!(unresolved.len(), 1);
        assert!(unresolved[0].reason.contains("ambiguous"));
    }

    #[test]
    fn prose_see_mention_rewrites() {
        let map = id_map(&[("activity-job", "ADR-047", "ADR-0041")]);
        let fbl = features_by_legacy(&map);
        let (out, rewrites, _) = sweep_file(
            "Per the previous decision, see ADR-047 for details.\n",
            Path::new("docs/design/activity-job/2_design.md"),
            Some("activity-job"),
            &map,
            &fbl,
        );
        assert!(out.contains("see [ADR-0041]"), "got: {out}");
        assert_eq!(rewrites.len(), 1);
    }

    #[test]
    fn cross_feature_explicit_ref_rewrites() {
        let map = id_map(&[("activity-job", "ADR-017", "ADR-0017")]);
        let fbl = features_by_legacy(&map);
        let (out, rewrites, _) = sweep_file(
            "See activity-job/ADR-017 for the parent rule.\n",
            Path::new("docs/design/auditability/2_design.md"),
            Some("auditability"),
            &map,
            &fbl,
        );
        assert!(out.contains("[ADR-0017]"), "got: {out}");
        assert_eq!(rewrites.len(), 1);
    }

    #[test]
    fn already_global_id_is_left_alone() {
        let map = id_map(&[("activity-job", "ADR-048", "ADR-0042")]);
        let fbl = features_by_legacy(&map);
        let (out, rewrites, _) = sweep_file(
            "See [ADR-0042] (global).\n",
            Path::new("docs/design/activity-job/2_design.md"),
            Some("activity-job"),
            &map,
            &fbl,
        );
        assert!(out.contains("[ADR-0042]"));
        assert!(rewrites.is_empty(), "should not rewrite global IDs");
    }

    #[test]
    fn code_fenced_block_content_is_not_rewritten() {
        // YAML examples like `legacy_ids: - activity-job/ADR-039` are literal
        // illustrative content, not references.
        let map = id_map(&[("activity-job", "ADR-039", "ADR-0042")]);
        let fbl = features_by_legacy(&map);
        let src = "\
Some prose with `activity-job/ADR-039` ref.

```yaml
legacy_ids:
  - activity-job/ADR-039
```

More prose.
";
        let (out, rewrites, _) = sweep_file(
            src,
            Path::new("docs/design/adr-artifact/2_design.md"),
            Some("adr-artifact"),
            &map,
            &fbl,
        );
        // Inside the fence the literal must survive verbatim.
        assert!(
            out.contains("  - activity-job/ADR-039"),
            "literal in fence rewritten: {out}"
        );
        // Outside-fence prose still gets rewritten — note that the prose
        // mention is inside inline backticks so it should *also* be left
        // alone (see backtick test). This test pins the fence behavior.
        // Rewrites should be 0 because both occurrences are protected.
        assert_eq!(rewrites.len(), 0, "unexpected rewrites: {rewrites:?}");
    }

    #[test]
    fn inline_backtick_content_is_not_rewritten() {
        // Example: `- legacy_id — set during migration (e.g. `"activity-job/ADR-017"`)`
        // The string inside backticks is a literal example of what a
        // legacy_id looks like, not a reference.
        let map = id_map(&[("activity-job", "ADR-017", "ADR-0099")]);
        let fbl = features_by_legacy(&map);
        let src = "- `legacy_id` — set during migration (e.g. `\"activity-job/ADR-017\"`) so it resolves\n";
        let (out, rewrites, _) = sweep_file(
            src,
            Path::new("docs/design/adr-artifact/1_overview.md"),
            Some("adr-artifact"),
            &map,
            &fbl,
        );
        assert!(
            out.contains("activity-job/ADR-017"),
            "inline-backtick literal rewritten: {out}"
        );
        assert!(rewrites.is_empty(), "unexpected rewrites: {rewrites:?}");
    }

    #[test]
    fn references_outside_code_spans_still_rewrite() {
        // Sanity: the fence/backtick guards must not break the happy path.
        let map = id_map(&[("activity-job", "ADR-048", "ADR-0042")]);
        let fbl = features_by_legacy(&map);
        let src = "See [ADR-048] for details.\n";
        let (out, rewrites, _) = sweep_file(
            src,
            Path::new("docs/design/activity-job/2_design.md"),
            Some("activity-job"),
            &map,
            &fbl,
        );
        assert!(out.contains("[ADR-0042]"), "should rewrite: {out}");
        assert_eq!(rewrites.len(), 1);
    }
}
