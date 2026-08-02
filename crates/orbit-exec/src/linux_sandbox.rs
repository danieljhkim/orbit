//! Linux Bubblewrap write-confinement for CLI-backed agents.
//!
//! The backend deliberately keeps the host filesystem readable and materializes
//! `ResolvedFsProfile::modify` as ordered bind mounts. It is therefore honest
//! write confinement, not a general read-policy implementation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use orbit_common::types::{OrbitError, ResolvedFsProfile};
use orbit_common::utility::glob::compile_glob_regex;
use orbit_common::utility::redaction::non_sensitive_env_vars;

const TRUSTED_BWRAP_PATH: &str = "/usr/bin/bwrap";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BwrapProbeOutcome {
    pub available: bool,
    pub trusted_path: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxBwrapPlan {
    pub wrapper: String,
    pub args: Vec<String>,
}

#[derive(Debug)]
pub struct LinuxBwrapSpawnRequest<'a> {
    pub plan: &'a LinuxBwrapPlan,
    pub env: &'a [(String, String)],
    pub cwd: Option<&'a Path>,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
}

/// Snapshot guard for Bubblewrap's one known write-policy gap: a
/// non-subtree deny glob cannot be represented for a path that does not exist
/// yet. Managed worktrees are disposable and single-writer, so Orbit records
/// existing matches before spawn and rejects any new matches after the child.
#[derive(Debug, Clone)]
pub struct LinuxBwrapPostRunGuard {
    rules: Vec<String>,
    before: BTreeSet<PathBuf>,
}

impl LinuxBwrapPostRunGuard {
    pub fn capture(profile: &ResolvedFsProfile) -> Result<Option<Self>, OrbitError> {
        let rules = non_subtree_denies(profile);
        if rules.is_empty() {
            return Ok(None);
        }
        let before = expand_rules(&rules)?;
        Ok(Some(Self { rules, before }))
    }

    pub fn verify(&self) -> Result<(), OrbitError> {
        let after = expand_rules(&self.rules)?;
        let created = after.difference(&self.before).next();
        if let Some(path) = created {
            return Err(OrbitError::PolicyDenied(format!(
                "linux-bwrap child created a path forbidden by denyModify before commit: {}",
                path.display()
            )));
        }
        Ok(())
    }
}

pub fn bwrap_program_for_audit() -> &'static str {
    TRUSTED_BWRAP_PATH
}

pub fn bwrap_path() -> Option<PathBuf> {
    let path = Path::new(TRUSTED_BWRAP_PATH);
    (cfg!(target_os = "linux") && is_executable(path)).then(|| path.to_path_buf())
}

pub fn bwrap_unavailable_message() -> String {
    format!("trusted Bubblewrap not available at {TRUSTED_BWRAP_PATH}")
}

/// Probe the namespaces and mounts Orbit relies on rather than treating a
/// present binary as usable. The host network namespace is retained
/// explicitly with `--share-net`.
pub fn probe_bwrap() -> BwrapProbeOutcome {
    let Some(path) = bwrap_path() else {
        return BwrapProbeOutcome {
            available: false,
            trusted_path: TRUSTED_BWRAP_PATH.to_string(),
            detail: bwrap_unavailable_message(),
        };
    };
    let args = base_namespace_args();
    let output = Command::new(&path)
        .args(&args)
        .arg("--")
        .arg("/bin/true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(output) if output.status.success() => BwrapProbeOutcome {
            available: true,
            trusted_path: path.display().to_string(),
            detail: "capability probe succeeded".to_string(),
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            BwrapProbeOutcome {
                available: false,
                trusted_path: path.display().to_string(),
                detail: format!(
                    "Bubblewrap capability probe failed{}{}",
                    if detail.is_empty() { "" } else { ": " },
                    detail
                ),
            }
        }
        Err(error) => BwrapProbeOutcome {
            available: false,
            trusted_path: path.display().to_string(),
            detail: format!("Bubblewrap capability probe could not execute: {error}"),
        },
    }
}

/// Compile a deterministic Bubblewrap argv. Broad writable roots are emitted
/// before every deny mount. A positive exact/subtree rule that is strictly
/// nested under an earlier deny is emitted in rule order as an explicit narrow
/// re-allow; positive ancestors and equal roots cannot override a deny.
pub fn compile_linux_bwrap_argv(
    profile: &ResolvedFsProfile,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    managed_worktree: bool,
) -> Result<LinuxBwrapPlan, OrbitError> {
    let mut out = base_namespace_args();
    // Replace mutable host pseudo-filesystems and scratch before applying
    // policy mounts. A writable worktree nested below `/tmp` is then re-bound
    // narrowly without exposing the rest of the host scratch tree.
    out.extend([
        "--dev".to_string(),
        "/dev".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--tmpfs".to_string(),
        "/tmp".to_string(),
    ]);
    let writable_roots = positive_mount_roots(profile)?;

    for (index, rule) in profile.modify.iter().enumerate() {
        if rule.starts_with('!') || is_narrow_reallow(&profile.modify[..index], rule) {
            continue;
        }
        for path in mount_paths_for_rule(rule, true)? {
            push_mount(&mut out, "--bind", &path);
        }
    }
    for (index, rule) in profile.modify.iter().enumerate() {
        if let Some(denied) = rule.strip_prefix('!') {
            if !is_exact_or_subtree(denied)
                && !managed_worktree
                && overlaps_writable_root(denied, &writable_roots)
            {
                return Err(OrbitError::PolicyDenied(format!(
                    "linux-bwrap cannot enforce non-subtree denyModify `{denied}` for a direct invocation; use a managed worktree"
                )));
            }
            for path in mount_paths_for_rule(denied, false)? {
                push_mount(&mut out, "--ro-bind", &path);
            }
        } else if is_narrow_reallow(&profile.modify[..index], rule) {
            for path in mount_paths_for_rule(rule, false)? {
                push_mount(&mut out, "--bind", &path);
            }
        }
    }

    if let Some(cwd) = cwd {
        let cwd = canonical_existing(cwd, "sandbox cwd")?;
        out.push("--chdir".to_string());
        out.push(cwd.display().to_string());
    }
    out.push("--".to_string());
    out.push(program.to_string());
    out.extend(args.iter().cloned());

    Ok(LinuxBwrapPlan {
        wrapper: TRUSTED_BWRAP_PATH.to_string(),
        args: out,
    })
}

pub fn spawn_under_linux_bwrap(request: LinuxBwrapSpawnRequest<'_>) -> Result<Child, OrbitError> {
    let LinuxBwrapSpawnRequest {
        plan,
        env,
        cwd,
        stdin,
        stdout,
        stderr,
    } = request;
    if plan.wrapper != TRUSTED_BWRAP_PATH {
        return Err(OrbitError::Execution(format!(
            "refusing untrusted Bubblewrap wrapper `{}`",
            plan.wrapper
        )));
    }
    let mut command = Command::new(TRUSTED_BWRAP_PATH);
    command
        .args(&plan.args)
        .env_clear()
        .envs(non_sensitive_env_vars())
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().map_err(|error| {
        OrbitError::Execution(format!(
            "failed to spawn trusted Bubblewrap `{TRUSTED_BWRAP_PATH}`: {error}"
        ))
    })
}

fn base_namespace_args() -> Vec<String> {
    [
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--share-net",
        "--ro-bind",
        "/",
        "/",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn push_mount(args: &mut Vec<String>, option: &str, path: &Path) {
    let rendered = path.display().to_string();
    args.push(option.to_string());
    args.push(rendered.clone());
    args.push(rendered);
}

fn positive_mount_roots(profile: &ResolvedFsProfile) -> Result<Vec<PathBuf>, OrbitError> {
    let mut roots = BTreeSet::new();
    for rule in profile.modify.iter().filter(|rule| !rule.starts_with('!')) {
        for root in mount_paths_for_rule(rule, false)? {
            roots.insert(root);
        }
    }
    Ok(roots.into_iter().collect())
}

fn mount_paths_for_rule(rule: &str, require_match: bool) -> Result<Vec<PathBuf>, OrbitError> {
    if is_exact_or_subtree(rule) {
        let root = rule.strip_suffix("/**").unwrap_or(rule);
        if !Path::new(root).exists() && !require_match {
            return Ok(Vec::new());
        }
        let path = canonical_existing(Path::new(root), "sandbox mount")?;
        return Ok(vec![path]);
    }
    let matches = expand_rule(rule)?;
    if require_match && matches.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "linux-bwrap modify rule `{rule}` has no existing path to mount"
        )));
    }
    Ok(matches.into_iter().collect())
}

fn is_narrow_reallow(prior_rules: &[String], rule: &str) -> bool {
    let Some(root) = exact_or_subtree_root(rule) else {
        return false;
    };
    prior_rules
        .iter()
        .filter_map(|prior| prior.strip_prefix('!'))
        .filter_map(exact_or_subtree_root)
        .any(|denied| root != denied && root.starts_with(&denied))
}

fn exact_or_subtree_root(rule: &str) -> Option<PathBuf> {
    is_exact_or_subtree(rule).then(|| PathBuf::from(rule.strip_suffix("/**").unwrap_or(rule)))
}

fn is_exact_or_subtree(rule: &str) -> bool {
    let body = rule.strip_suffix("/**").unwrap_or(rule);
    !body.contains(['*', '?'])
}

fn overlaps_writable_root(rule: &str, roots: &[PathBuf]) -> bool {
    let prefix = static_prefix(rule);
    roots
        .iter()
        .any(|root| root.starts_with(&prefix) || prefix.starts_with(root))
}

fn non_subtree_denies(profile: &ResolvedFsProfile) -> Vec<String> {
    profile
        .modify
        .iter()
        .filter_map(|rule| rule.strip_prefix('!'))
        .filter(|rule| !is_exact_or_subtree(rule))
        .map(str::to_string)
        .collect()
}

fn expand_rules(rules: &[String]) -> Result<BTreeSet<PathBuf>, OrbitError> {
    let mut expanded = BTreeSet::new();
    for rule in rules {
        expanded.extend(expand_rule(rule)?);
    }
    Ok(expanded)
}

fn expand_rule(rule: &str) -> Result<BTreeSet<PathBuf>, OrbitError> {
    let regex = compile_glob_regex(rule).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "invalid linux-bwrap filesystem glob `{rule}`: {error}"
        ))
    })?;
    let root = nearest_existing_ancestor(&static_prefix(rule))?;
    let mut candidates = Vec::new();
    walk_paths(&root, &mut candidates)?;
    let mut matches = BTreeSet::new();
    for candidate in candidates {
        let rendered = candidate.to_string_lossy().replace('\\', "/");
        if regex.is_match(&rendered) {
            matches.insert(canonical_existing(&candidate, "denyModify match")?);
        }
    }
    Ok(matches)
}

fn static_prefix(rule: &str) -> PathBuf {
    let wildcard = rule.find(['*', '?']).unwrap_or(rule.len());
    let literal = &rule[..wildcard];
    let boundary = literal.rfind('/').unwrap_or(0);
    let prefix = if wildcard == rule.len() {
        literal
    } else if boundary == 0 {
        "/"
    } else {
        &literal[..boundary]
    };
    PathBuf::from(prefix)
}

fn nearest_existing_ancestor(path: &Path) -> Result<PathBuf, OrbitError> {
    let mut current = path.to_path_buf();
    while !current.exists() {
        if !current.pop() {
            return Err(OrbitError::InvalidInput(format!(
                "linux-bwrap glob root `{}` has no existing ancestor",
                path.display()
            )));
        }
    }
    canonical_existing(&current, "glob search root")
}

fn walk_paths(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), OrbitError> {
    out.push(root.to_path_buf());
    if !root.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(root).map_err(|error| {
        OrbitError::Execution(format!(
            "read linux-bwrap glob root `{}`: {error}",
            root.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            OrbitError::Execution(format!("read linux-bwrap glob entry: {error}"))
        })?;
        let path = entry.path();
        out.push(path.clone());
        if entry
            .file_type()
            .map_err(|error| {
                OrbitError::Execution(format!("inspect `{}`: {error}", path.display()))
            })?
            .is_dir()
        {
            walk_paths(&path, out)?;
        }
    }
    Ok(())
}

fn canonical_existing(path: &Path, label: &str) -> Result<PathBuf, OrbitError> {
    path.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "{label} `{}` must exist and resolve canonically: {error}",
            path.display()
        ))
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
