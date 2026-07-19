//! Routine discovery [ORB-10021]: enumerate the global workspace registry,
//! visit every registered, active workspace whose versioned config declares
//! `[routines] role = "source"` (ADR-0205), and load `.orbit/routines/*.yaml`
//! from each — fail-closed per file. An invalid definition becomes a load
//! error and that routine is treated as absent; it never fires with defaults.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    OrbitError, RoutineDefinition, Workspace, parse_local_routine_yaml, parse_routine_yaml,
};

use crate::OrbitRuntime;

use super::due::parse_cron;

/// Directory under a source workspace's `.orbit/` holding routine YAML files.
pub const ROUTINES_DIR: &str = "routines";

/// Subdirectory of [`ROUTINES_DIR`] holding machine-local routine definitions
/// (gitignored by convention). The directory is the origin contract — the
/// sweep never shells out to `git check-ignore` (host-registry design §6).
pub const LOCAL_ROUTINES_SUBDIR: &str = "local";

/// Where a routine definition came from — the directory decides, not git
/// status (host-registry design §6). Committed definitions must pin a host;
/// local definitions are implicitly pinned to the loading host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineOrigin {
    /// A git-committed definition under `.orbit/routines/` (excluding
    /// `local/`). Requires a non-empty explicit `hosts:` pin.
    Committed,
    /// A machine-local definition under `.orbit/routines/local/`. Implicitly
    /// pinned to the loading host; may not name another host.
    Local,
}

impl RoutineOrigin {
    /// Stable lowercase label for reporting (`committed` / `local`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Local => "local",
        }
    }
}

/// One successfully loaded, fully validated routine.
#[derive(Debug, Clone)]
pub struct LoadedRoutine {
    /// The parsed definition.
    pub definition: RoutineDefinition,
    /// Whether the definition is committed or machine-local.
    pub origin: RoutineOrigin,
    /// Registry name of the source workspace.
    pub source_workspace: String,
    /// The source workspace's `.orbit` directory (dispatch root).
    pub source_orbit_dir: PathBuf,
    /// Path of the YAML file the definition came from.
    pub path: PathBuf,
}

/// One fail-closed load failure, kept for reporting: the routine (or source)
/// it names is treated as absent this pass.
#[derive(Debug, Clone)]
pub struct RoutineLoadError {
    /// Registry name of the source workspace involved.
    pub source_workspace: String,
    /// File that failed, when the failure is file-scoped.
    pub path: Option<PathBuf>,
    /// Human-readable reason.
    pub message: String,
}

/// Result of one discovery pass across all routine sources.
#[derive(Debug, Default)]
pub struct RoutineCollection {
    /// Valid routines, in stable (workspace, filename) order.
    pub routines: Vec<LoadedRoutine>,
    /// Everything that failed fail-closed.
    pub errors: Vec<RoutineLoadError>,
}

/// Registered workspaces with their runtimes, ready for routine discovery
/// and dispatch, plus the workspaces that failed to open (reported loudly —
/// registry hygiene must not silently shrink the source set).
#[derive(Default)]
pub struct DiscoveredWorkspaces {
    /// Active, openable workspaces (source or not) with their runtimes.
    pub entries: Vec<(Workspace, OrbitRuntime)>,
    /// Registered workspaces that could not be opened.
    pub errors: Vec<RoutineLoadError>,
}

/// Registry-neutral source of workspace runtimes for routine status and sweep.
/// Implementations may consult a catalog, but Core only observes the prepared
/// workspaces and fail-closed discovery errors.
pub trait RoutineWorkspaceProvider {
    fn discover_workspaces(&self, global_root: &Path) -> Result<DiscoveredWorkspaces, OrbitError>;
}

/// Load routines from every source workspace among `workspaces` (the same
/// runtimes are later used for dispatch), origin-aware: committed definitions
/// under `.orbit/routines/` require a host pin, local definitions under
/// `.orbit/routines/local/` are implicit to `host_id`. Cross-origin name
/// collisions are load-time errors: every colliding definition is dropped and
/// each conflicting source is named.
pub fn collect_routines(
    workspaces: &[(Workspace, OrbitRuntime)],
    host_id: &str,
) -> RoutineCollection {
    let mut collection = RoutineCollection::default();

    for (workspace, runtime) in workspaces {
        if !runtime.routines_source() {
            continue;
        }
        load_source_workspace(workspace, runtime, host_id, &mut collection);
    }

    drop_name_collisions(&mut collection);
    collection
}

fn load_source_workspace(
    workspace: &Workspace,
    runtime: &OrbitRuntime,
    host_id: &str,
    collection: &mut RoutineCollection,
) {
    let routines_dir = runtime.shared_root().join(ROUTINES_DIR);
    if !routines_dir.is_dir() {
        // A source with no routines directory is simply an empty source.
        return;
    }

    // Committed definitions: top-level YAML files. The `local/` subdirectory is
    // a directory (never a file) so it is skipped here and scanned separately.
    load_origin_dir(
        &routines_dir,
        RoutineOrigin::Committed,
        workspace,
        runtime,
        host_id,
        collection,
    );

    // Local definitions: `.orbit/routines/local/`, implicit to this host.
    let local_dir = routines_dir.join(LOCAL_ROUTINES_SUBDIR);
    if local_dir.is_dir() {
        load_origin_dir(
            &local_dir,
            RoutineOrigin::Local,
            workspace,
            runtime,
            host_id,
            collection,
        );
    }
}

/// Load every top-level YAML file in `dir` under `origin`. Only regular files
/// are considered, so a committed scan of `.orbit/routines/` never treats the
/// `local/` subdirectory as a definition.
fn load_origin_dir(
    dir: &Path,
    origin: RoutineOrigin,
    workspace: &Workspace,
    runtime: &OrbitRuntime,
    host_id: &str,
    collection: &mut RoutineCollection,
) {
    let paths = match yaml_files_in(dir) {
        Ok(paths) => paths,
        Err(error) => {
            collection.errors.push(RoutineLoadError {
                source_workspace: workspace.name.clone(),
                path: Some(dir.to_path_buf()),
                message: format!("failed to list routines directory: {error}"),
            });
            return;
        }
    };

    for path in paths {
        match load_routine_file(&path, origin, workspace, runtime, host_id) {
            Ok(routine) => collection.routines.push(routine),
            Err(message) => collection.errors.push(RoutineLoadError {
                source_workspace: workspace.name.clone(),
                path: Some(path),
                message,
            }),
        }
    }
}

/// Regular `*.yaml` / `*.yml` files directly in `dir`, in stable filename
/// order. Subdirectories (e.g. `local/` under the committed scan) are skipped.
fn yaml_files_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml")
                })
        })
        .collect();
    paths.sort();
    Ok(paths)
}

fn load_routine_file(
    path: &Path,
    origin: RoutineOrigin,
    workspace: &Workspace,
    runtime: &OrbitRuntime,
    host_id: &str,
) -> Result<LoadedRoutine, String> {
    let raw = std::fs::read_to_string(path).map_err(|error| format!("read failed: {error}"))?;
    // Origin decides the host contract: committed definitions must pin a host,
    // local definitions are implicit to (and may name only) this host.
    let definition = match origin {
        RoutineOrigin::Committed => parse_routine_yaml(&raw).map_err(|error| error.to_string())?,
        RoutineOrigin::Local => {
            parse_local_routine_yaml(&raw, host_id).map_err(|error| error.to_string())?
        }
    };

    // Load-time cron validation: a routine with an unparsable trigger never
    // reaches the due computation.
    parse_cron(&definition.trigger.cron).map_err(|error| error.to_string())?;

    // Load-time target resolution through the source workspace's catalog,
    // like `target:` steps in JobV2: an unresolvable target is a load error,
    // not a fire-time surprise (ADR-0206).
    let job_name = definition.target.job_name();
    runtime.load_v2_job_asset_by_name(job_name).map_err(|_| {
        format!(
            "target 'job:{job_name}' does not resolve in workspace '{}': \
             no such job in its catalog",
            workspace.name
        )
    })?;

    Ok(LoadedRoutine {
        definition,
        origin,
        source_workspace: workspace.name.clone(),
        source_orbit_dir: runtime.shared_root(),
        path: path.to_path_buf(),
    })
}

/// Names must be unique across every routine source *and origin* on a host; a
/// collision is a load-time error and every colliding definition is treated as
/// absent (fail-closed — firing an arbitrary winner would make behavior depend
/// on iteration order, and a committed and a local definition must never
/// silently shadow one another). Each colliding definition's error names all
/// conflicting sources so both origins are visible.
fn drop_name_collisions(collection: &mut RoutineCollection) {
    // Collect a stable, sorted descriptor of every source per name.
    let mut sources_by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for routine in &collection.routines {
        sources_by_name
            .entry(routine.definition.name.clone())
            .or_default()
            .push(format!(
                "{} ({} origin, workspace '{}')",
                routine.path.display(),
                routine.origin.as_str(),
                routine.source_workspace
            ));
    }
    let colliding: BTreeMap<String, Vec<String>> = sources_by_name
        .into_iter()
        .filter_map(|(name, mut sources)| {
            if sources.len() > 1 {
                sources.sort();
                Some((name, sources))
            } else {
                None
            }
        })
        .collect();
    if colliding.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(collection.routines.len());
    for routine in collection.routines.drain(..) {
        if let Some(sources) = colliding.get(&routine.definition.name) {
            collection.errors.push(RoutineLoadError {
                source_workspace: routine.source_workspace.clone(),
                path: Some(routine.path.clone()),
                message: format!(
                    "routine name '{}' is defined by more than one source; names must be \
                     unique across committed and local origins on a host — defined at: {}",
                    routine.definition.name,
                    sources.join("; ")
                ),
            });
        } else {
            kept.push(routine);
        }
    }
    collection.routines = kept;
}
