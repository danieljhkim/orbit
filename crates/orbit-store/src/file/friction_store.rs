use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::friction::DEFAULT_FRICTION_TAGS;
use orbit_common::types::{
    FrictionFrontmatter, FrictionRecord, OrbitError, Task, TaskStatus, all_agent_families,
    normalize_optional_attribution_label, resolve_agent_model_pair,
};
use orbit_common::utility::fs::{atomic_write_text, with_exclusive_file_lock};
use serde_json::{Value, json};

const TAGS_FILENAME: &str = "tags.yaml";

#[derive(Debug, Clone)]
pub struct FrictionAddParams {
    pub model: String,
    pub body: String,
    pub tags: Vec<String>,
    pub during_task: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct FrictionListFilter {
    pub model: Option<String>,
    pub tag: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct StoredFrictionRecord {
    pub record: FrictionRecord,
    pub path: PathBuf,
}

pub fn add_friction(
    frictions_root: &Path,
    params: FrictionAddParams,
) -> Result<StoredFrictionRecord, OrbitError> {
    validate_model(&params.model)?;
    let taxonomy = load_tag_taxonomy(frictions_root)?;
    let tags = normalize_and_validate_tags(params.tags, &taxonomy)?;
    let month = params.created_at.format("%Y-%m").to_string();
    let month_dir = frictions_root.join(&month);
    let lock_target = month_dir.join(".allocation");
    with_exclusive_file_lock(&lock_target, "friction id allocation", || {
        fs::create_dir_all(&month_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
        let next = next_month_counter(&month_dir)?;
        let id = format!("F{month}-{next:03}");
        let path = month_dir.join(format!("F{next:03}.md"));
        if path.exists() {
            return Err(OrbitError::Store(format!(
                "friction record already exists: {}",
                path.display()
            )));
        }
        write_record_at(
            &path,
            &FrictionRecord {
                id,
                model: params.model.trim().to_string(),
                created_at: params.created_at,
                tags,
                during_task: params.during_task,
                body: params.body,
            },
        )
    })
}

pub fn list_frictions(
    frictions_root: &Path,
    filter: &FrictionListFilter,
) -> Result<Vec<StoredFrictionRecord>, OrbitError> {
    if !frictions_root.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for path in friction_record_paths(frictions_root)? {
        let stored = read_record_at(&path)?;
        if filter
            .model
            .as_deref()
            .is_some_and(|model| stored.record.model != model)
        {
            continue;
        }
        if filter
            .tag
            .as_deref()
            .is_some_and(|tag| !stored.record.tags.iter().any(|value| value == tag))
        {
            continue;
        }
        if filter
            .from
            .is_some_and(|from| stored.record.created_at < from)
        {
            continue;
        }
        if filter.to.is_some_and(|to| stored.record.created_at > to) {
            continue;
        }
        records.push(stored);
    }
    records.sort_by(|left, right| {
        left.record
            .created_at
            .cmp(&right.record.created_at)
            .then_with(|| left.record.id.cmp(&right.record.id))
    });
    Ok(records)
}

pub fn show_friction(
    frictions_root: &Path,
    id: &str,
) -> Result<Option<StoredFrictionRecord>, OrbitError> {
    validate_friction_id(id)?;
    let month = &id[1..8];
    let nnn = &id[9..12];
    let path = frictions_root.join(month).join(format!("F{nnn}.md"));
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_record_at(&path)?))
}

pub fn friction_stats(frictions_root: &Path, tasks: &[Task]) -> Result<Value, OrbitError> {
    let records = list_frictions(frictions_root, &FrictionListFilter::default())?;
    let tasks_done = completed_tasks_by_model(tasks);
    let mut frictions_by_model: BTreeMap<String, u64> = BTreeMap::new();
    let mut frictions_by_tag_model: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut models = BTreeSet::new();

    for stored in &records {
        models.insert(stored.record.model.clone());
        *frictions_by_model
            .entry(stored.record.model.clone())
            .or_insert(0) += 1;
        for tag in &stored.record.tags {
            *frictions_by_tag_model
                .entry(tag.clone())
                .or_default()
                .entry(stored.record.model.clone())
                .or_insert(0) += 1;
        }
    }
    models.extend(tasks_done.keys().cloned());
    models.extend(known_family_model_keys());

    let mut by_model = serde_json::Map::new();
    for model in &models {
        let frictions = frictions_by_model.get(model).copied().unwrap_or(0);
        let done = tasks_done.get(model).copied().unwrap_or(0);
        by_model.insert(model.clone(), rate_row(frictions, done));
    }

    let mut by_tag = serde_json::Map::new();
    for (tag, by_model_counts) in frictions_by_tag_model {
        let mut tag_map = serde_json::Map::new();
        for model in &models {
            let frictions = by_model_counts.get(model).copied().unwrap_or(0);
            let done = tasks_done.get(model).copied().unwrap_or(0);
            tag_map.insert(model.clone(), rate_row(frictions, done));
        }
        by_tag.insert(tag, Value::Object(tag_map));
    }

    Ok(json!({
        "by_model": Value::Object(by_model),
        "by_tag": Value::Object(by_tag),
    }))
}

pub fn ensure_default_tag_taxonomy(frictions_root: &Path) -> Result<PathBuf, OrbitError> {
    let path = frictions_root.join(TAGS_FILENAME);
    if !path.exists() {
        let mut body = String::new();
        for (tag, description) in DEFAULT_FRICTION_TAGS {
            body.push_str(&format!("{tag}: \"{description}\"\n"));
        }
        atomic_write_text(&path, &body).map_err(|error| OrbitError::Io(error.to_string()))?;
    }
    Ok(path)
}

fn load_tag_taxonomy(frictions_root: &Path) -> Result<BTreeSet<String>, OrbitError> {
    let path = ensure_default_tag_taxonomy(frictions_root)?;
    let raw = fs::read_to_string(&path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    let value: serde_yaml::Value = serde_yaml::from_str(&raw)
        .map_err(|error| OrbitError::InvalidInput(format!("parse {}: {error}", path.display())))?;
    let mut tags = BTreeSet::new();
    collect_tags_from_yaml(&value, &mut tags);
    if tags.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "{} must define at least one friction tag",
            path.display()
        )));
    }
    Ok(tags)
}

fn collect_tags_from_yaml(value: &serde_yaml::Value, out: &mut BTreeSet<String>) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            if let Some(tags_value) = map.get(serde_yaml::Value::String("tags".to_string())) {
                collect_tags_from_yaml(tags_value, out);
                return;
            }
            for key in map.keys() {
                if let Some(tag) = key.as_str().and_then(normalize_tag) {
                    out.insert(tag);
                }
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                if let Some(tag) = item.as_str().and_then(normalize_tag) {
                    out.insert(tag);
                }
            }
        }
        _ => {}
    }
}

fn normalize_and_validate_tags(
    raw_tags: Vec<String>,
    taxonomy: &BTreeSet<String>,
) -> Result<Vec<String>, OrbitError> {
    let mut tags = BTreeSet::new();
    for raw in raw_tags {
        if let Some(tag) = normalize_tag(&raw) {
            tags.insert(tag);
        }
    }
    if tags.is_empty() {
        tags.insert("other".to_string());
    }
    let invalid = tags
        .iter()
        .filter(|tag| !taxonomy.contains(*tag))
        .cloned()
        .collect::<Vec<_>>();
    if !invalid.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "unknown friction tag(s): {}. valid tags: {}",
            invalid.join(", "),
            taxonomy.iter().cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(tags.into_iter().collect())
}

fn normalize_tag(raw: &str) -> Option<String> {
    let value = raw.trim().to_ascii_lowercase();
    if value.is_empty() { None } else { Some(value) }
}

fn validate_model(model: &str) -> Result<(), OrbitError> {
    if model.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "friction model must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn next_month_counter(month_dir: &Path) -> Result<u32, OrbitError> {
    let mut max_seen = 0;
    if month_dir.exists() {
        for entry in fs::read_dir(month_dir).map_err(|error| OrbitError::Io(error.to_string()))? {
            let path = entry
                .map_err(|error| OrbitError::Io(error.to_string()))?
                .path();
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if file_name.len() == 7
                && file_name.starts_with('F')
                && file_name.ends_with(".md")
                && let Ok(value) = file_name[1..4].parse::<u32>()
            {
                max_seen = max_seen.max(value);
            }
        }
    }
    Ok(max_seen + 1)
}

fn write_record_at(
    path: &Path,
    record: &FrictionRecord,
) -> Result<StoredFrictionRecord, OrbitError> {
    let frontmatter = FrictionFrontmatter {
        id: record.id.clone(),
        model: record.model.clone(),
        created_at: record.created_at,
        tags: record.tags.clone(),
        during_task: record.during_task.clone(),
    };
    let yaml = serde_yaml::to_string(&frontmatter)
        .map_err(|error| OrbitError::Store(format!("serialize friction frontmatter: {error}")))?;
    let content = format!("---\n{}---\n{}\n", yaml, record.body.trim_end());
    atomic_write_text(path, &content).map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(StoredFrictionRecord {
        record: record.clone(),
        path: path.to_path_buf(),
    })
}

fn read_record_at(path: &Path) -> Result<StoredFrictionRecord, OrbitError> {
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    let (yaml, body) = split_frontmatter(&raw).ok_or_else(|| {
        OrbitError::Store(format!(
            "friction record {} must start with YAML frontmatter",
            path.display()
        ))
    })?;
    let frontmatter: FrictionFrontmatter = serde_yaml::from_str(yaml).map_err(|error| {
        OrbitError::Store(format!(
            "parse friction frontmatter {}: {error}",
            path.display()
        ))
    })?;
    Ok(StoredFrictionRecord {
        record: FrictionRecord {
            id: frontmatter.id,
            model: frontmatter.model,
            created_at: frontmatter.created_at,
            tags: frontmatter.tags,
            during_task: frontmatter.during_task,
            body: body.trim_start_matches('\n').trim_end().to_string(),
        },
        path: path.to_path_buf(),
    })
}

fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("---\n")?;
    let (yaml, body) = rest.split_once("\n---\n")?;
    Some((yaml, body))
}

fn friction_record_paths(frictions_root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut paths = Vec::new();
    for month_entry in
        fs::read_dir(frictions_root).map_err(|error| OrbitError::Io(error.to_string()))?
    {
        let month_path = month_entry
            .map_err(|error| OrbitError::Io(error.to_string()))?
            .path();
        if !month_path.is_dir() {
            continue;
        }
        for record_entry in
            fs::read_dir(&month_path).map_err(|error| OrbitError::Io(error.to_string()))?
        {
            let path = record_entry
                .map_err(|error| OrbitError::Io(error.to_string()))?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("md") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn validate_friction_id(id: &str) -> Result<(), OrbitError> {
    let bytes = id.as_bytes();
    let valid = bytes.len() == 12
        && bytes[0] == b'F'
        && bytes[5] == b'-'
        && bytes[8] == b'-'
        && bytes[1..5].iter().all(u8::is_ascii_digit)
        && bytes[6..8].iter().all(u8::is_ascii_digit)
        && bytes[9..12].iter().all(u8::is_ascii_digit);
    if valid {
        Ok(())
    } else {
        Err(OrbitError::InvalidInput(format!(
            "friction id must match FYYYY-MM-NNN, got '{id}'"
        )))
    }
}

fn completed_tasks_by_model(tasks: &[Task]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for task in tasks {
        if !matches!(task.status, TaskStatus::Done | TaskStatus::Archived) {
            continue;
        }
        let Some(model) = normalize_optional_attribution_label(
            task.implemented_by.as_deref(),
            task.implemented_by.as_deref(),
        ) else {
            continue;
        };
        *counts.entry(model).or_insert(0) += 1;
    }
    counts
}

fn known_family_model_keys() -> impl Iterator<Item = String> {
    all_agent_families().into_iter().map(|family| {
        resolve_agent_model_pair(family)
            .map(|pair| pair.orchestrator)
            .unwrap_or_else(|| family.to_string())
    })
}

fn rate_row(frictions: u64, tasks_done: u64) -> Value {
    let rate = if tasks_done == 0 {
        json!("n/a")
    } else {
        let raw = (frictions as f64) * 10.0 / (tasks_done as f64);
        json!((raw * 10.0).round() / 10.0)
    };
    json!({
        "frictions": frictions,
        "tasks_done": tasks_done,
        "frictions_per_10_tasks": rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use orbit_common::types::{TaskPriority, TaskType};

    #[test]
    fn id_allocation_resets_across_month_boundary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let may = Utc.with_ymd_and_hms(2026, 5, 31, 23, 59, 0).unwrap();
        let june = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        let first = add_friction(root, params("gpt-5.5", may, vec!["tooling"])).expect("first add");
        let second = add_friction(root, params("gpt-5.5", may, vec!["docs"])).expect("second add");
        let next_month =
            add_friction(root, params("gpt-5.5", june, vec!["build"])).expect("next month add");

        assert_eq!(first.record.id, "F2026-05-001");
        assert_eq!(second.record.id, "F2026-05-002");
        assert_eq!(next_month.record.id, "F2026-06-001");
    }

    #[test]
    fn tag_validation_uses_taxonomy_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        ensure_default_tag_taxonomy(root).expect("taxonomy");
        let err = add_friction(root, params("gpt-5.5", Utc::now(), vec!["surprise-tag"]))
            .expect_err("unknown tag fails");
        assert!(err.to_string().contains("valid tags"), "{err}");

        fs::write(root.join(TAGS_FILENAME), "surprise-tag: allowed\n").expect("rewrite taxonomy");
        add_friction(root, params("gpt-5.5", Utc::now(), vec!["surprise-tag"]))
            .expect("new taxonomy tag succeeds");
    }

    #[test]
    fn stats_render_zero_task_model_rate_as_na() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        add_friction(root, params("gpt-zero", Utc::now(), vec!["tooling"])).expect("add friction");
        let mut done = task("T1", TaskStatus::Done);
        done.implemented_by = Some("gpt-done".to_string());

        let stats = friction_stats(root, &[done]).expect("stats");
        assert_eq!(
            stats["by_model"]["gpt-zero"]["frictions_per_10_tasks"],
            json!("n/a")
        );
        assert_eq!(
            stats["by_model"]["gpt-done"]["frictions_per_10_tasks"],
            json!(0.0)
        );
    }

    #[test]
    fn stats_render_zero_rows_for_known_grok_family() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();

        let stats = friction_stats(root, &[]).expect("stats");

        assert_eq!(stats["by_model"]["grok-4"]["frictions"], json!(0));
        assert_eq!(stats["by_model"]["grok-4"]["tasks_done"], json!(0));
        assert_eq!(
            stats["by_model"]["grok-4"]["frictions_per_10_tasks"],
            json!("n/a")
        );
    }

    fn params(model: &str, created_at: DateTime<Utc>, tags: Vec<&str>) -> FrictionAddParams {
        FrictionAddParams {
            model: model.to_string(),
            body: "Body".to_string(),
            tags: tags.into_iter().map(str::to_string).collect(),
            during_task: None,
            created_at,
        }
    }

    fn task(id: &str, status: TaskStatus) -> Task {
        let now = Utc.with_ymd_and_hms(2026, 5, 10, 0, 0, 0).unwrap();
        Task {
            id: id.to_string(),
            title: id.to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            created_by: None,
            planned_by: None,
            implemented_by: None,
            status,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_status: None,
            external_refs: Vec::new(),
            relations: Vec::new(),
            job_run_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}
