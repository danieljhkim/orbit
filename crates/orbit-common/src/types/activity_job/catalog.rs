//! v2 activity catalog + `target: activity:<name>` resolution (Phase 4).
//!
//! A catalog is a name → `ActivityV2` map built from one or more directory
//! trees of v2 YAML assets. [`resolve_job_target_refs`] walks a [`JobV2`]
//! DAG and rewrites every [`JobV2StepBody::TargetRef`] into
//! [`JobV2StepBody::Target`] by looking up the named activity in the
//! catalog. Resolution runs after [`super::backend::resolve_job_backends`]
//! (so the `Auto` → concrete rewrite also applies to the newly-inlined
//! specs) and before [`super::backend::validate_job_loop_session_backends`].
//!
//! Scope resolution (§9 `MergeByKey`) is intentionally not implemented here:
//! callers (orbit-core entry points) supply the already-merged directory
//! list, and duplicate names are rejected by [`V2ActivityCatalog::insert`]
//! so scope-merge decisions stay outside the types crate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::activity_v2::ActivityV2;
use super::asset_loader::{AssetLoadError, load_activity_asset};
use super::job_v2::{JobV2, JobV2Step, JobV2StepBody, LoopBlock, TargetRef, TargetStep};
use super::tool_allowlist::{
    ToolAllowlistError, validate_activity_tool_allowlist_against_registered_tools,
};

/// `activity:<name>` prefix for the `target:` field on a [`TargetRef`].
pub const ACTIVITY_REF_PREFIX: &str = "activity:";

#[derive(Debug, Default, Clone)]
pub struct V2ActivityCatalog {
    entries: BTreeMap<String, ActivityV2>,
    sources: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("read dir {path}: {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("read file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: AssetLoadError,
    },
    #[error("duplicate activity name `{name}` — defined in both {first} and {second}")]
    DuplicateName {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
    #[error("activity `{name}` tool allowlist invalid: {source}")]
    ToolAllowlist {
        name: String,
        source: ToolAllowlistError,
    },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResolveError {
    #[error("step `{step_id}`: target `{target}` does not start with `activity:` prefix")]
    UnknownRefKind { step_id: String, target: String },
    #[error("step `{step_id}`: activity `{name}` not found in catalog")]
    ActivityNotInCatalog { step_id: String, name: String },
    #[error("job recovery_activity `{name}` not found in catalog")]
    RecoveryActivityNotInCatalog { name: String },
    #[error("step `{step_id}`: recovery_activity `{name}` not found in catalog")]
    StepRecoveryActivityNotInCatalog { step_id: String, name: String },
}

impl V2ActivityCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load every `*.yaml` / `*.yml` file under `dir` (recursively) as a
    /// schemaVersion 2 activity asset. Duplicate names across files are a
    /// hard error; merge semantics belong to the caller.
    pub fn load_dir(&mut self, dir: &Path) -> Result<(), CatalogError> {
        self.load_dir_inner(dir, false, ExistingNamePolicy::Reject)
            .map(|_| ())
    }

    /// Variant of [`load_dir`] that skips retired schemaVersion 1 assets and
    /// returns the file paths that were ignored.
    pub fn load_dir_skipping_retired(&mut self, dir: &Path) -> Result<Vec<PathBuf>, CatalogError> {
        self.load_dir_inner(dir, true, ExistingNamePolicy::Reject)
    }

    /// Layered-catalog variant of [`load_dir_skipping_retired`]. Duplicate
    /// names inside `dir` are still invalid, but names that already exist in
    /// the catalog are left untouched so callers can load directories from
    /// highest to lowest precedence.
    pub fn load_dir_skipping_retired_prefer_existing(
        &mut self,
        dir: &Path,
    ) -> Result<Vec<PathBuf>, CatalogError> {
        self.load_dir_inner(dir, true, ExistingNamePolicy::PreferExisting)
    }

    fn load_dir_inner(
        &mut self,
        dir: &Path,
        skip_retired: bool,
        existing_name_policy: ExistingNamePolicy,
    ) -> Result<Vec<PathBuf>, CatalogError> {
        let mut local_entries: BTreeMap<String, (ActivityV2, PathBuf)> = BTreeMap::new();
        let mut skipped = Vec::new();
        walk_dir(dir, &mut |path| {
            let yaml = std::fs::read_to_string(path).map_err(|source| CatalogError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
            let asset = match load_activity_asset(&yaml) {
                Ok(asset) => asset,
                Err(_source @ AssetLoadError::RetiredVersion(_)) if skip_retired => {
                    skipped.push(path.to_path_buf());
                    return Ok(());
                }
                Err(source) => {
                    return Err(CatalogError::Parse {
                        path: path.to_path_buf(),
                        source,
                    });
                }
            };
            if let Some((_, prev)) = local_entries.get(&asset.name) {
                return Err(CatalogError::DuplicateName {
                    name: asset.name,
                    first: prev.clone(),
                    second: path.to_path_buf(),
                });
            }
            local_entries.insert(asset.name, (asset.spec, path.to_path_buf()));
            Ok(())
        })?;

        for (name, (spec, path)) in local_entries {
            if let Some(prev) = self.sources.get(&name) {
                match existing_name_policy {
                    ExistingNamePolicy::Reject => {
                        return Err(CatalogError::DuplicateName {
                            name,
                            first: prev.clone(),
                            second: path,
                        });
                    }
                    ExistingNamePolicy::PreferExisting => continue,
                }
            }
            self.sources.insert(name.clone(), path);
            self.entries.insert(name, spec);
        }

        Ok(skipped)
    }

    /// Insert an explicit `(name, spec)` pair — used by smokes and in-memory
    /// composition. Returns the displaced entry if the name was already set.
    pub fn insert(&mut self, name: impl Into<String>, spec: ActivityV2) -> Option<ActivityV2> {
        let name = name.into();
        self.sources
            .insert(name.clone(), PathBuf::from("<explicit>"));
        self.entries.insert(name, spec)
    }

    pub fn get(&self, name: &str) -> Option<&ActivityV2> {
        self.entries.get(name)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Validate every agent-facing activity tool allowlist against a caller
    /// supplied registry snapshot. This keeps `orbit-common` registry-agnostic
    /// while letting core/engine fail malformed assets before dispatch.
    pub fn validate_tool_allowlists<'a, I>(&self, registered_tools: I) -> Result<(), CatalogError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let registered_tools: Vec<&str> = registered_tools.into_iter().collect();
        for (name, activity) in &self.entries {
            validate_activity_tool_allowlist_against_registered_tools(
                activity,
                registered_tools.iter().copied(),
            )
            .map_err(|source| CatalogError::ToolAllowlist {
                name: name.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum ExistingNamePolicy {
    Reject,
    PreferExisting,
}

fn walk_dir(
    dir: &Path,
    cb: &mut dyn FnMut(&Path) -> Result<(), CatalogError>,
) -> Result<(), CatalogError> {
    let iter = std::fs::read_dir(dir).map_err(|source| CatalogError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in iter {
        let entry = entry.map_err(|source| CatalogError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, cb)?;
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e == "yaml" || e == "yml");
        if is_yaml {
            cb(&path)?;
        }
    }
    Ok(())
}

/// Walk `job` and rewrite every [`JobV2StepBody::TargetRef`] into a
/// [`JobV2StepBody::Target`] using the named [`ActivityV2`] from `catalog`.
/// A ref with an unknown name is a hard error; silently succeeding would
/// leave a `TargetRef` lurking past dispatch where the executor would
/// ignore it.
pub fn resolve_job_target_refs(
    job: &mut JobV2,
    catalog: &V2ActivityCatalog,
) -> Result<(), ResolveError> {
    job.resolved_recovery_activity = match job.recovery_activity.as_deref() {
        Some(name) => Some(catalog.get(name).cloned().ok_or_else(|| {
            ResolveError::RecoveryActivityNotInCatalog {
                name: name.to_string(),
            }
        })?),
        None => None,
    };

    for step in &mut job.steps {
        resolve_step(step, catalog)?;
    }
    Ok(())
}

fn resolve_step(step: &mut JobV2Step, catalog: &V2ActivityCatalog) -> Result<(), ResolveError> {
    step.resolved_recovery_activity = match step.recovery_activity.as_deref() {
        Some(name) => Some(catalog.get(name).cloned().ok_or_else(|| {
            ResolveError::StepRecoveryActivityNotInCatalog {
                step_id: step.id.clone(),
                name: name.to_string(),
            }
        })?),
        None => None,
    };

    match &mut step.body {
        JobV2StepBody::Target(_) => Ok(()),
        JobV2StepBody::TargetRef(_) => {
            // Swap the body out so we can own the ref without cloning; the
            // replacement is a throwaway `Target` that we immediately
            // overwrite with the resolved one.
            let old = std::mem::replace(
                &mut step.body,
                JobV2StepBody::TargetRef(TargetRef {
                    target: String::new(),
                    default_input: None,
                    timeout_seconds: 0,
                    session: None,
                    role: None,
                }),
            );
            let JobV2StepBody::TargetRef(r) = old else {
                unreachable!("checked above");
            };
            let resolved = resolve_ref(&step.id, r, catalog)?;
            step.body = JobV2StepBody::Target(resolved);
            Ok(())
        }
        JobV2StepBody::Parallel { parallel } => {
            for branch in &mut parallel.branches {
                resolve_step(branch, catalog)?;
            }
            Ok(())
        }
        JobV2StepBody::FanOut { fan_out, .. } => resolve_step(&mut fan_out.worker, catalog),
        JobV2StepBody::Loop { loop_ } => resolve_loop(loop_, catalog),
    }
}

fn resolve_loop(block: &mut LoopBlock, catalog: &V2ActivityCatalog) -> Result<(), ResolveError> {
    for step in &mut block.steps {
        resolve_step(step, catalog)?;
    }
    Ok(())
}

fn resolve_ref(
    step_id: &str,
    r: TargetRef,
    catalog: &V2ActivityCatalog,
) -> Result<TargetStep, ResolveError> {
    let name =
        r.target
            .strip_prefix(ACTIVITY_REF_PREFIX)
            .ok_or_else(|| ResolveError::UnknownRefKind {
                step_id: step_id.to_string(),
                target: r.target.clone(),
            })?;
    let activity = catalog
        .get(name)
        .ok_or_else(|| ResolveError::ActivityNotInCatalog {
            step_id: step_id.to_string(),
            name: name.to_string(),
        })?;
    Ok(TargetStep {
        spec: activity.spec.clone(),
        activity_name: Some(name.to_string()),
        fs_profile: activity.fs_profile.clone(),
        default_input: r.default_input,
        timeout_seconds: r.timeout_seconds,
        session: r.session,
        role: r.role,
    })
}
