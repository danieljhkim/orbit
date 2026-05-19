use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use chrono::{NaiveDate, Utc};
use orbit_common::types::{NotFoundKind, OrbitError};
use orbit_common::utility::fs::atomic_write_text;
use regex::{Captures, Regex};
use serde::Serialize;

pub const DESIGN_CONVENTIONS_TEMPLATE: &str =
    include_str!("../../../../docs/design/CONVENTIONS.md");

const DESIGN_DIR: &str = "docs/design";
const NUMBERED_DOCS: [(&str, &str); 4] = [
    ("1_overview.md", "Overview"),
    ("2_design.md", "Design"),
    ("3_vision.md", "Vision"),
    ("4_decisions.md", "Decisions"),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DesignFeatureSummary {
    pub feature: String,
    pub docs: BTreeMap<String, DesignDocInfo>,
    pub specs_path: PathBuf,
    pub references_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DesignDocInfo {
    pub path: PathBuf,
    pub owner: Option<String>,
    pub last_updated: Option<NaiveDate>,
    pub decay_status: DesignDecayStatus,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesignDecayStatus {
    Fresh,
    Stale,
}

pub fn init_feature(
    repo_root: &Path,
    feature: &str,
    owner: &str,
) -> Result<DesignFeatureSummary, OrbitError> {
    let feature = validate_feature_name(feature)?;
    let owner = normalize_owner(owner);
    let feature_dir = repo_root.join(DESIGN_DIR).join(&feature);
    if feature_dir.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "design feature already exists: {}",
            feature_dir.display()
        )));
    }

    fs::create_dir_all(feature_dir.join("specs"))
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    fs::create_dir_all(feature_dir.join("references"))
        .map_err(|error| OrbitError::Io(error.to_string()))?;

    let today = Utc::now().date_naive();
    let title = titleize_feature(&feature);
    for (file_name, role) in NUMBERED_DOCS {
        let content = scaffold_doc(&feature, &title, role, &owner, today);
        atomic_write_text(&feature_dir.join(file_name), &content)
            .map_err(|error| OrbitError::Io(error.to_string()))?;
    }

    show_feature(repo_root, &feature)
}

pub fn list_features(repo_root: &Path) -> Result<Vec<DesignFeatureSummary>, OrbitError> {
    let design_root = repo_root.join(DESIGN_DIR);
    if !design_root.exists() {
        return Ok(Vec::new());
    }

    let mut features = Vec::new();
    for entry in fs::read_dir(&design_root).map_err(|error| OrbitError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| OrbitError::Io(error.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let feature = entry.file_name().to_string_lossy().into_owned();
        if feature.starts_with('_') {
            continue;
        }
        features.push(show_feature(repo_root, &feature)?);
    }
    features.sort_by(|left, right| left.feature.cmp(&right.feature));
    Ok(features)
}

pub fn show_feature(repo_root: &Path, feature: &str) -> Result<DesignFeatureSummary, OrbitError> {
    let feature = validate_feature_name(feature)?;
    let feature_dir = repo_root.join(DESIGN_DIR).join(&feature);
    if !feature_dir.is_dir() {
        return Err(OrbitError::not_found(NotFoundKind::DesignFeature, feature));
    }

    let mut docs = BTreeMap::new();
    for (file_name, _role) in NUMBERED_DOCS {
        let path = feature_dir.join(file_name);
        let owner = doc_owner(&path)?;
        let last_updated = doc_last_updated(repo_root, &path)?;
        let decay_status = if doc_is_stale(repo_root, &path, last_updated)? {
            DesignDecayStatus::Stale
        } else {
            DesignDecayStatus::Fresh
        };
        docs.insert(
            file_name.to_string(),
            DesignDocInfo {
                path: absolute_path(&path)?,
                owner,
                last_updated,
                decay_status,
            },
        );
    }

    Ok(DesignFeatureSummary {
        feature,
        docs,
        specs_path: absolute_path(&feature_dir.join("specs"))?,
        references_path: absolute_path(&feature_dir.join("references"))?,
    })
}

pub fn seed_design_conventions(repo_root: &Path, owner: &str) -> Result<bool, OrbitError> {
    let conventions_path = repo_root.join(DESIGN_DIR).join("CONVENTIONS.md");
    if conventions_path.exists() {
        return Ok(false);
    }
    let owner = normalize_owner(owner);
    let content = conventions_with_owner(&owner)?;
    atomic_write_text(&conventions_path, &content)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(true)
}

fn doc_last_updated(repo_root: &Path, doc: &Path) -> Result<Option<NaiveDate>, OrbitError> {
    if let Some(date) = declared_last_updated(doc)? {
        return Ok(Some(date));
    }
    git_last_commit_date(repo_root, doc)
}

fn declared_last_updated(doc: &Path) -> Result<Option<NaiveDate>, OrbitError> {
    let body = fs::read_to_string(doc).map_err(|error| OrbitError::Io(error.to_string()))?;
    let Some(captures) = last_updated_regex()?.captures(&body) else {
        return Ok(None);
    };
    let Some(raw) = captures.get(1).map(|value| value.as_str()) else {
        return Ok(None);
    };
    Ok(NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok())
}

fn doc_owner(doc: &Path) -> Result<Option<String>, OrbitError> {
    if !doc.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(doc).map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(owner_regex()?
        .captures(&body)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty()))
}

fn extract_refs(doc: &Path) -> Result<BTreeSet<String>, OrbitError> {
    let body = fs::read_to_string(doc).map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(ref_regex()?
        .find_iter(&body)
        .map(|matched| matched.as_str().to_string())
        .collect())
}

fn doc_is_stale(
    repo_root: &Path,
    doc: &Path,
    last_updated: Option<NaiveDate>,
) -> Result<bool, OrbitError> {
    let Some(doc_date) = last_updated else {
        return Ok(false);
    };
    for reference in extract_refs(doc)? {
        let ref_path = repo_root.join(reference);
        if !ref_path.exists() {
            continue;
        }
        if let Some(ref_date) = git_last_commit_date(repo_root, &ref_path)?
            && ref_date > doc_date
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn git_last_commit_date(repo_root: &Path, path: &Path) -> Result<Option<NaiveDate>, OrbitError> {
    let path_arg = path.strip_prefix(repo_root).unwrap_or(path);
    let output = Command::new("git")
        .args(["log", "-1", "--format=%cs", "--"])
        .arg(path_arg)
        .current_dir(repo_root)
        .output()
        .map_err(|error| OrbitError::Execution(format!("run git log: {error}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw = stdout.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok())
}

fn scaffold_doc(
    feature: &str,
    title: &str,
    role: &str,
    owner: &str,
    last_updated: NaiveDate,
) -> String {
    let (doc_role, role_body) = match role {
        "Overview" => (
            "overview",
            "\n<Feature overview.>\n\n## 1. Motivation\n\n## 2. Core Concepts\n\n## 3. At a Glance\n\n| Concern | File | Task |\n|---------|------|------|\n",
        ),
        "Design" => (
            "design",
            "\n<Scope of the current implementation.>\n\n## 1. Current Implementation\n\n## 2. Concerns & Honest Limitations\n",
        ),
        "Vision" => (
            "vision",
            "\n<Scope of the forward-looking design.>\n\n## 1. Open Questions\n\n## 2. Prior Work\n\n## 3. What May Be Distinctive\n\n## 4. References\n",
        ),
        "Decisions" => (
            "decisions",
            "\nADR entries are append-only and ordered ascending.\n",
        ),
        _ => ("unknown", "\n"),
    };
    format!(
        "---\ntitle: \"{title} — {role}\"\nowner: {owner}\nlast_updated: {last_updated}\nstatus: Draft\nfeature: {feature}\ndoc_role: {doc_role}\n---\n\n# {title} — {role}\n{role_body}\n## Task References\n\nResolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.\n"
    )
}

fn conventions_with_owner(owner: &str) -> Result<String, OrbitError> {
    Ok(owner_regex()?
        .replacen(DESIGN_CONVENTIONS_TEMPLATE, 1, |captures: &Captures<'_>| {
            let matched = captures
                .get(0)
                .map(|value| value.as_str().trim_start())
                .unwrap_or_default();
            if matched.starts_with("owner:") {
                format!("owner: {owner}")
            } else {
                format!("**Owner:** {owner}")
            }
        })
        .to_string())
}

fn validate_feature_name(feature: &str) -> Result<String, OrbitError> {
    let feature = feature.trim();
    if feature_regex()?.is_match(feature) {
        Ok(feature.to_string())
    } else {
        Err(OrbitError::InvalidInput(format!(
            "feature must be lowercase, hyphenated, and path-safe: {feature}"
        )))
    }
}

fn normalize_owner(owner: &str) -> String {
    let owner = owner.trim();
    if owner.is_empty() {
        "human".to_string()
    } else {
        owner.to_string()
    }
}

fn titleize_feature(feature: &str) -> String {
    feature
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn absolute_path(path: &Path) -> Result<PathBuf, OrbitError> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .map_err(|error| OrbitError::Io(error.to_string()))?
        .join(path))
}

fn last_updated_regex() -> Result<&'static Regex, OrbitError> {
    static REGEX: OnceLock<Result<Regex, String>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r#"(?m)^\s*(?:\*\*Last updated:\*\*|last_updated:)\s*"?(\d{4}-\d{2}-\d{2})"?\s*$"#,
            )
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| OrbitError::Execution(format!("compile last-updated regex: {error}")))
}

fn owner_regex() -> Result<&'static Regex, OrbitError> {
    static REGEX: OnceLock<Result<Regex, String>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(r"(?m)^\s*(?:\*\*Owner:\*\*|owner:)\s*(.*?)\s*$")
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| OrbitError::Execution(format!("compile owner regex: {error}")))
}

fn ref_regex() -> Result<&'static Regex, OrbitError> {
    static REGEX: OnceLock<Result<Regex, String>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(r"crates/[a-zA-Z0-9_/.\-]+\.rs").map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| OrbitError::Execution(format!("compile reference regex: {error}")))
}

fn feature_regex() -> Result<&'static Regex, OrbitError> {
    static REGEX: OnceLock<Result<Regex, String>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| OrbitError::Execution(format!("compile feature regex: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_feature_creates_exact_layout_with_frontmatter() {
        let root = tempdir().expect("tempdir");
        let summary = init_feature(root.path(), "design-docs", "codex").expect("init feature");

        assert_eq!(summary.feature, "design-docs");
        let feature_dir = root.path().join(DESIGN_DIR).join("design-docs");
        let mut entries = fs::read_dir(&feature_dir)
            .expect("read feature dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(
            entries,
            vec![
                "1_overview.md",
                "2_design.md",
                "3_vision.md",
                "4_decisions.md",
                "references",
                "specs",
            ]
        );
        assert!(
            fs::read_dir(feature_dir.join("specs"))
                .expect("read specs")
                .next()
                .is_none()
        );
        assert!(
            fs::read_dir(feature_dir.join("references"))
                .expect("read references")
                .next()
                .is_none()
        );
        for (file_name, _) in NUMBERED_DOCS {
            let content = fs::read_to_string(feature_dir.join(file_name)).expect("read doc");
            assert!(content.starts_with("---\n"));
            assert!(content.contains("status: Draft"));
            assert!(content.contains("owner: codex"));
            assert!(content.contains("feature: design-docs"));
            assert!(last_updated_regex().unwrap().is_match(&content));
        }
    }

    #[test]
    fn init_feature_rejects_existing_folder() {
        let root = tempdir().expect("tempdir");
        init_feature(root.path(), "design-docs", "codex").expect("first init");
        let error = init_feature(root.path(), "design-docs", "codex").expect_err("reject");
        assert!(matches!(error, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn seed_design_conventions_rewrites_owner_once() {
        let root = tempdir().expect("tempdir");
        let seeded = seed_design_conventions(root.path(), "codex").expect("seed conventions");
        assert!(seeded);
        let conventions = fs::read_to_string(root.path().join(DESIGN_DIR).join("CONVENTIONS.md"))
            .expect("read conventions");
        assert!(conventions.contains("owner: codex"));
        assert!(!conventions.contains("owner: daniel"));

        let second = seed_design_conventions(root.path(), "claude").expect("idempotent");
        assert!(!second);
        let conventions = fs::read_to_string(root.path().join(DESIGN_DIR).join("CONVENTIONS.md"))
            .expect("read conventions again");
        assert!(conventions.contains("owner: codex"));
    }
}
