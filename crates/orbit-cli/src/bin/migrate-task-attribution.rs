use std::fs;
use std::path::{Path, PathBuf};

use orbit_core::{OrbitError, OrbitRuntime};
use orbit_types::{all_agent_families, normalize_attribution_label};
use regex::{Captures, Regex};
use serde_json::{Map, Value};

struct NormalizationRules {
    legacy_label: Regex,
    ownership_line: Regex,
    model_line: Regex,
}

#[derive(Default)]
struct MigrationStats {
    task_files: usize,
    scoreboard_files: usize,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), OrbitError> {
    let mut args = std::env::args().skip(1);
    let mut root_override: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                let Some(value) = args.next() else {
                    return Err(OrbitError::InvalidInput(
                        "expected a path after --root".to_string(),
                    ));
                };
                root_override = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => {
                return Err(OrbitError::InvalidInput(format!(
                    "unknown argument: {other}"
                )));
            }
        }
    }

    let runtime = OrbitRuntime::initialize_with_root_override(root_override.as_deref())?;
    let rules = build_rules()?;
    let data_root = runtime.data_root();
    let mut stats = MigrationStats::default();

    stats.task_files = normalize_task_artifacts(&data_root.join("tasks"), &rules)?;
    stats.scoreboard_files =
        normalize_scoreboard_artifacts(&data_root.join("state/scoreboard"), &rules)?;

    let summary_path = runtime.scoreboard_summary_path();
    let previous_summary = fs::read_to_string(&summary_path).ok();
    runtime.generate_scoreboard_summary()?;
    let next_summary = fs::read_to_string(&summary_path).ok();
    if previous_summary != next_summary {
        stats.scoreboard_files += 1;
    }

    println!(
        "normalized {} task files and {} scoreboard files",
        stats.task_files, stats.scoreboard_files
    );
    Ok(())
}

fn build_rules() -> Result<NormalizationRules, OrbitError> {
    let families = all_agent_families()
        .iter()
        .map(|family| regex::escape(family))
        .collect::<Vec<_>>()
        .join("|");
    let legacy_label = Regex::new(&format!(
        r"(?i)\b(?:agent|{families})\s*/\s*[A-Za-z0-9][A-Za-z0-9._:-]*"
    ))
    .map_err(|e| OrbitError::Execution(format!("compile legacy label regex: {e}")))?;
    let ownership_line = Regex::new(
        r"^(?P<indent>\s*)(?P<field>created_by|planned_by|implemented_by|assigned_to|proposed_by):\s*(?P<value>[^#\n]+?)\s*$",
    )
    .map_err(|e| OrbitError::Execution(format!("compile ownership regex: {e}")))?;
    let model_line = Regex::new(r"^(?P<indent>\s*)model:\s*(?P<value>[^#\n]+?)\s*$")
        .map_err(|e| OrbitError::Execution(format!("compile model regex: {e}")))?;
    Ok(NormalizationRules {
        legacy_label,
        ownership_line,
        model_line,
    })
}

fn normalize_task_artifacts(
    tasks_dir: &Path,
    rules: &NormalizationRules,
) -> Result<usize, OrbitError> {
    if !tasks_dir.exists() {
        return Ok(0);
    }

    let mut changed = 0usize;
    visit_files(tasks_dir, &mut |path| {
        let file_changed = match path.file_name().and_then(|name| name.to_str()) {
            Some("task.yaml") => normalize_task_yaml(path, rules)?,
            _ => normalize_text_file(path, &rules.legacy_label)?,
        };
        if file_changed {
            changed += 1;
        }
        Ok(())
    })?;
    Ok(changed)
}

fn normalize_task_yaml(path: &Path, rules: &NormalizationRules) -> Result<bool, OrbitError> {
    let Some(raw) = read_optional_utf8(path)? else {
        return Ok(false);
    };

    let model_hint = infer_task_model_hint(&raw, rules);
    let mut normalized = String::with_capacity(raw.len());
    for segment in raw.split_inclusive('\n') {
        let (line, newline) = if let Some(stripped) = segment.strip_suffix('\n') {
            (stripped, "\n")
        } else {
            (segment, "")
        };
        normalized.push_str(&normalize_task_yaml_line(
            line,
            rules,
            model_hint.as_deref(),
        ));
        normalized.push_str(newline);
    }

    if normalized == raw {
        return Ok(false);
    }

    write_atomic_text(path, &normalized)?;
    Ok(true)
}

fn infer_task_model_hint(task_yaml: &str, rules: &NormalizationRules) -> Option<String> {
    for line in task_yaml.lines() {
        if let Some(captures) = rules.model_line.captures(line) {
            let candidate =
                normalize_attribution_label(parse_scalar(captures.name("value")?.as_str())?, None);
            if is_model_like(&candidate) {
                return Some(candidate);
            }
        }
    }

    for line in task_yaml.lines() {
        if let Some(captures) = rules.ownership_line.captures(line) {
            let candidate =
                normalize_attribution_label(parse_scalar(captures.name("value")?.as_str())?, None);
            if is_model_like(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn normalize_task_yaml_line(
    line: &str,
    rules: &NormalizationRules,
    model_hint: Option<&str>,
) -> String {
    if line.starts_with("agent:") || line.starts_with("actor_identity:") {
        return line.to_string();
    }

    if let Some(captures) = rules.model_line.captures(line) {
        let indent = captures.name("indent").map_or("", |m| m.as_str());
        let value = captures.name("value").map_or("", |m| m.as_str());
        if parse_scalar(value).is_none()
            && let Some(model_hint) = model_hint.filter(|value| is_model_like(value))
        {
            return format!("{indent}model: {model_hint}");
        }
    }

    if let Some(captures) = rules.ownership_line.captures(line) {
        let indent = captures.name("indent").map_or("", |m| m.as_str());
        let field = captures.name("field").map_or("", |m| m.as_str());
        let value = captures.name("value").map_or("", |m| m.as_str());
        if let Some(scalar) = parse_scalar(value) {
            let normalized = normalize_attribution_label(scalar, model_hint);
            if normalized != scalar {
                return format!("{indent}{field}: {normalized}");
            }
        }
    }

    normalize_legacy_text(line, &rules.legacy_label)
}

fn normalize_text_file(path: &Path, legacy_label: &Regex) -> Result<bool, OrbitError> {
    let Some(raw) = read_optional_utf8(path)? else {
        return Ok(false);
    };
    let normalized = normalize_legacy_text(&raw, legacy_label);
    if normalized == raw {
        return Ok(false);
    }
    write_atomic_text(path, &normalized)?;
    Ok(true)
}

fn normalize_legacy_text(text: &str, legacy_label: &Regex) -> String {
    legacy_label
        .replace_all(text, |captures: &Captures<'_>| {
            normalize_attribution_label(captures.get(0).map_or("", |m| m.as_str()), None)
        })
        .into_owned()
}

fn normalize_scoreboard_artifacts(
    scoreboard_dir: &Path,
    rules: &NormalizationRules,
) -> Result<usize, OrbitError> {
    if !scoreboard_dir.exists() {
        return Ok(0);
    }

    let mut changed = 0usize;
    for file_name in ["pr.json", "friction_bounty.json"] {
        if normalize_counter_scoreboard(&scoreboard_dir.join(file_name))? {
            changed += 1;
        }
    }
    if normalize_tokens_scoreboard(&scoreboard_dir.join("tokens.json"), rules)? {
        changed += 1;
    }
    Ok(changed)
}

fn normalize_counter_scoreboard(path: &Path) -> Result<bool, OrbitError> {
    if !path.exists() {
        return Ok(false);
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| OrbitError::Io(format!("read {}: {e}", path.display())))?;
    if raw.trim().is_empty() {
        return Ok(false);
    }

    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| OrbitError::Io(format!("parse {}: {e}", path.display())))?;
    let Value::Object(metrics) = parsed else {
        return Err(OrbitError::Io(format!(
            "{} must be a JSON object",
            path.display()
        )));
    };

    let mut normalized = Map::new();
    for (metric, value) in metrics {
        let Value::Object(entries) = value else {
            continue;
        };
        let mut normalized_entries = Map::new();
        for (model, count) in entries {
            let Some(count) = count.as_u64() else {
                continue;
            };
            let model = normalize_attribution_label(&model, None);
            let total = normalized_entries
                .get(&model)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_add(count);
            normalized_entries.insert(model, Value::from(total));
        }
        normalized.insert(metric, Value::Object(normalized_entries));
    }

    write_json_if_changed(path, &raw, Value::Object(normalized))
}

fn normalize_tokens_scoreboard(
    path: &Path,
    rules: &NormalizationRules,
) -> Result<bool, OrbitError> {
    if !path.exists() {
        return Ok(false);
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| OrbitError::Io(format!("read {}: {e}", path.display())))?;
    if raw.trim().is_empty() {
        return Ok(false);
    }

    let mut parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| OrbitError::Io(format!("parse {}: {e}", path.display())))?;
    let changed = normalize_tokens_value(&mut parsed, rules);
    if !changed {
        return Ok(false);
    }

    write_json_if_changed(path, &raw, parsed)
}

fn normalize_tokens_value(value: &mut Value, rules: &NormalizationRules) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            for (key, nested) in map.iter_mut() {
                if key == "model"
                    && let Some(raw) = nested.as_str()
                {
                    let normalized = normalize_attribution_label(raw, None);
                    if normalized != raw {
                        *nested = Value::String(normalized);
                        changed = true;
                        continue;
                    }
                }
                changed |= normalize_tokens_value(nested, rules);
            }
            changed
        }
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
            normalize_tokens_value(item, rules) || changed
        }),
        Value::String(text) => {
            let normalized = normalize_legacy_text(text, &rules.legacy_label);
            if normalized != *text {
                *text = normalized;
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn write_json_if_changed(path: &Path, original: &str, value: Value) -> Result<bool, OrbitError> {
    let normalized = serde_json::to_string_pretty(&value)
        .map_err(|e| OrbitError::Io(format!("serialize {}: {e}", path.display())))?;
    let normalized = format!("{normalized}\n");
    if normalized == original {
        return Ok(false);
    }
    write_atomic_text(path, &normalized)?;
    Ok(true)
}

fn visit_files(
    dir: &Path,
    visitor: &mut impl FnMut(&Path) -> Result<(), OrbitError>,
) -> Result<(), OrbitError> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(format!("read_dir {}: {e}", dir.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| OrbitError::Io(format!("read_dir {}: {e}", dir.display())))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            visit_files(&path, visitor)?;
        } else if path.is_file() {
            visitor(&path)?;
        }
    }

    Ok(())
}

fn read_optional_utf8(path: &Path) -> Result<Option<String>, OrbitError> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => Ok(None),
        Err(err) => Err(OrbitError::Io(format!("read {}: {err}", path.display()))),
    }
}

fn write_atomic_text(path: &Path, content: &str) -> Result<(), OrbitError> {
    let parent = path
        .parent()
        .ok_or_else(|| OrbitError::Io(format!("no parent dir for {}", path.display())))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| OrbitError::Io(format!("invalid file name {}", path.display())))?;
    let tmp_path = parent.join(format!(".{file_name}.tmp"));
    fs::write(&tmp_path, content)
        .map_err(|e| OrbitError::Io(format!("write {}: {e}", tmp_path.display())))?;
    fs::rename(&tmp_path, path)
        .map_err(|e| OrbitError::Io(format!("rename {}: {e}", path.display())))?;
    Ok(())
}

fn parse_scalar(value: &str) -> Option<&str> {
    let trimmed = value.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
        None
    } else {
        Some(trimmed)
    }
}

fn is_model_like(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("agent")
        && !trimmed.eq_ignore_ascii_case("human")
        && !trimmed.eq_ignore_ascii_case("system")
        && !all_agent_families()
            .iter()
            .any(|family| trimmed.eq_ignore_ascii_case(family))
}

fn print_help() {
    println!("Usage: migrate-task-attribution [--root <path>]");
    println!("Rewrites Orbit task and scoreboard artifacts to the model-only attribution schema.");
}
