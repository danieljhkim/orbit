//! The destination's statement about who may call it, and as what [ORB-11052].
//!
//! [`CALLERS_FILE`] is the mirror of the federated destinations file:
//! destinations declare who this machine may call, callers declare who may
//! call this machine and with which capabilities. Both are machine-global
//! operator files; neither belongs in a workspace.
//!
//! # Why a destination-side file at all
//!
//! On an SSH destination the caller writes the remote argv, so `--operator` on
//! `orbit mcp serve` was a caller-authored grant: anyone with shell access
//! stamped their own session `Operator` and satisfied every governed
//! operation. This module moves the *statement* to the machine that executes
//! the work — the caller's argv becomes a request, and the file is the
//! ceiling.
//!
//! # What this is not
//!
//! The caller identity keyed on here is self-asserted: `--remote-caller-machine-id`
//! is a label the caller chooses, so a caller that can reach this destination
//! can also name a different row. That makes this an accident guard, in
//! keeping with the governance kernel's doctrine — strictly stronger than a
//! caller-authored grant, and not a boundary anything may be relaxed against.
//! Binding the identity to the key sshd already authenticated is separate,
//! deliberately later work.

use std::collections::{BTreeSet, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use orbit_common::OrbitError;
use orbit_types::identity::validate_machine_id;
use orbit_types::tool::{McpCapability, RemoteCallerGrant};
use serde::Deserialize;

use super::identity::McpSessionAuthority;

pub const CALLERS_FILE: &str = "mcp-callers.toml";

/// How the file is named in operator-facing text.
///
/// Denials and warnings quote the path the operator would type, not the
/// expanded home directory of whichever account the destination runs as.
pub const CALLERS_FILE_DISPLAY: &str = "~/.orbit/mcp-callers.toml";

/// What a destination serves a caller that matches no row.
///
/// Operator is deliberately absent: a default that could grant it would make
/// the escalation this file exists to close reachable by omitting a row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DefaultGrant {
    #[default]
    Agent,
    Deny,
}

impl DefaultGrant {
    fn capabilities(self) -> BTreeSet<McpCapability> {
        match self {
            Self::Agent => BTreeSet::from([McpCapability::Agent]),
            Self::Deny => BTreeSet::new(),
        }
    }
}

/// One caller this destination has agreed to serve.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallerRow {
    /// The calling machine's stable `hm_…` identity.
    pub machine_id: String,
    /// The ceiling this destination serves that caller. `agent` and
    /// `operator` only; `runner` is stamped in-process by a managed run and
    /// can never arrive over a transport.
    pub capabilities: Vec<String>,
    /// Operator-facing display name. Never an identity input.
    #[serde(default)]
    pub label: Option<String>,
    /// Narrows the grant to these logical `ws_*` IDs.
    #[serde(default)]
    pub workspaces: Option<Vec<String>>,
    /// Binds the row to an authenticated key. Parsed and surfaced; verifying
    /// it needs the forced-command path that supplies the key, which is not
    /// yet built [ORB-11053].
    #[serde(default)]
    pub ssh_key_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallersFile {
    #[serde(default)]
    pub default: DefaultGrant,
    #[serde(default)]
    pub callers: Vec<CallerRow>,
}

pub fn callers_path(global_orbit_root: &Path) -> PathBuf {
    global_orbit_root.join(CALLERS_FILE)
}

/// Load and validate the callers file.
///
/// A missing file is valid and means `default = "agent"` with no rows. A
/// malformed one is never served as if absent: it fails the whole file closed
/// here, at load, before any session is served.
pub fn load_callers(path: &Path) -> Result<CallersFile, OrbitError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CallersFile::default());
        }
        Err(error) => {
            return Err(OrbitError::Io(format!(
                "failed to read MCP callers '{}': {error}",
                path.display()
            )));
        }
    };
    let file: CallersFile = toml::from_str(&contents).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid MCP callers '{}': {error}", path.display()))
    })?;
    validate_callers(&file, path)?;
    Ok(file)
}

fn validate_callers(file: &CallersFile, path: &Path) -> Result<(), OrbitError> {
    let mut seen = HashSet::with_capacity(file.callers.len());
    for row in &file.callers {
        if !seen.insert(row.machine_id.as_str()) {
            return Err(OrbitError::AmbiguousCaller(format!(
                "machine_id '{}' appears more than once in '{}'",
                row.machine_id,
                path.display()
            )));
        }
    }
    for row in &file.callers {
        validate_machine_id(&row.machine_id).map_err(|error| {
            invalid(
                path,
                format!("invalid machine_id '{}': {error}", row.machine_id),
            )
        })?;
        parse_capabilities(row, path)?;
        if let Some(workspaces) = &row.workspaces {
            if workspaces.is_empty() {
                return Err(invalid(
                    path,
                    format!(
                        "caller '{}' has an empty `workspaces` list; omit the key to grant every \
                         workspace on this destination",
                        row.machine_id
                    ),
                ));
            }
            for workspace in workspaces {
                if !workspace.starts_with("ws_") {
                    return Err(invalid(
                        path,
                        format!(
                            "caller '{}' narrows to '{workspace}', which is not a logical \
                             workspace ID; `workspaces` takes `ws_*` IDs",
                            row.machine_id
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The row's capabilities, as the grantable subset of the capability
/// vocabulary.
///
/// `runner` parses as a capability everywhere else in Orbit, which is exactly
/// why it is rejected by name here rather than left to fall through an unknown
/// value: a run's own sanction must not be reachable over a transport.
fn parse_capabilities(row: &CallerRow, path: &Path) -> Result<BTreeSet<McpCapability>, OrbitError> {
    if row.capabilities.is_empty() {
        return Err(invalid(
            path,
            format!(
                "caller '{}' has an empty `capabilities` list; use `default = \"deny\"` or remove \
                 the row",
                row.machine_id
            ),
        ));
    }
    row.capabilities
        .iter()
        .map(|capability| match capability.as_str() {
            "agent" => Ok(McpCapability::Agent),
            "operator" => Ok(McpCapability::Operator),
            other => Err(invalid(
                path,
                format!(
                    "caller '{}' declares capability '{other}'; only `agent` and `operator` may \
                     be granted to a caller",
                    row.machine_id
                ),
            )),
        })
        .collect()
}

fn invalid(path: &Path, detail: String) -> OrbitError {
    OrbitError::InvalidInput(format!(
        "invalid MCP callers '{}': {detail}",
        path.display()
    ))
}

/// What this destination will serve one caller, before the caller's request is
/// taken into account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCallerGrant {
    /// The caller identity this grant was resolved for.
    pub caller_machine_id: String,
    /// The row's `label`, when one matched.
    pub label: Option<String>,
    /// Capabilities on the workspaces this grant covers.
    pub granted: BTreeSet<McpCapability>,
    /// Capabilities anywhere the grant does not cover — the file default.
    pub elsewhere: BTreeSet<McpCapability>,
    /// The `ws_*` IDs [`Self::granted`] applies to. `None` means every
    /// workspace on this destination.
    pub workspaces: Option<BTreeSet<String>>,
    /// Whether a row matched, or the file default answered.
    pub matched: bool,
}

impl ResolvedCallerGrant {
    /// The grant that applies to a call landing in `workspace_id`.
    ///
    /// A narrowed row falls back to the file default outside its listed
    /// workspaces rather than to a fixed `agent`, so `default = "deny"` still
    /// denies there. `None` — a call that resolved no workspace — takes the
    /// unnarrowed grant: every governed operation is workspace-scoped, so a
    /// workspace-less call is a discovery call the narrowing has nothing to
    /// say about.
    pub fn for_workspace(&self, workspace_id: Option<&str>) -> BTreeSet<McpCapability> {
        match (&self.workspaces, workspace_id) {
            (Some(covered), Some(workspace_id)) if !covered.contains(workspace_id) => {
                self.elsewhere.clone()
            }
            _ => self.granted.clone(),
        }
    }
}

impl CallersFile {
    /// What this destination serves `caller_machine_id`.
    ///
    /// An absent, malformed, or unmatched caller label falls to the file
    /// default. It never falls back to the caller's argv: that is the
    /// escalation being closed.
    pub fn resolve(&self, caller_machine_id: &str) -> ResolvedCallerGrant {
        let default = self.default.capabilities();
        let Some(row) = self
            .callers
            .iter()
            .find(|row| row.machine_id == caller_machine_id)
        else {
            return ResolvedCallerGrant {
                caller_machine_id: caller_machine_id.to_string(),
                label: None,
                granted: default.clone(),
                elsewhere: default,
                workspaces: None,
                matched: false,
            };
        };
        // Validated at load, so an unparseable capability here cannot reach a
        // served session; treat it as the default rather than panicking.
        let granted = row
            .capabilities
            .iter()
            .filter_map(|capability| capability.parse::<McpCapability>().ok())
            .filter(|capability| *capability != McpCapability::Runner)
            .collect::<BTreeSet<_>>();
        ResolvedCallerGrant {
            caller_machine_id: caller_machine_id.to_string(),
            label: row.label.clone(),
            granted,
            elsewhere: default,
            workspaces: row
                .workspaces
                .as_ref()
                .map(|workspaces| workspaces.iter().cloned().collect()),
            matched: true,
        }
    }
}

/// Whether this server process was started by sshd for a non-interactive
/// session.
///
/// Both halves are load-bearing. `SSH_CONNECTION` is set by sshd in the server
/// process, so a caller can neither forge it nor — the part that matters —
/// omit it; keying on `--remote-caller-machine-id` instead would let a caller
/// present a remote session as a local one by dropping the flag. The
/// non-terminal check separates the MCP transport, whose argv is `ssh -T`,
/// from a person who SSH'd in and started a server by hand.
pub fn remote_originated() -> bool {
    use std::io::IsTerminal;
    std::env::var("SSH_CONNECTION").is_ok_and(|value| !value.trim().is_empty())
        && !std::io::stdin().is_terminal()
}

/// The capabilities one MCP session may hold, and where they came from.
///
/// Built once at session establishment. A local session carries no grant and
/// keeps today's argv authority byte for byte; a remote-originated one carries
/// the destination's grant and is capped by it on every call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCapabilityPolicy {
    requested: BTreeSet<McpCapability>,
    grant: Option<ResolvedCallerGrant>,
}

impl SessionCapabilityPolicy {
    /// A session whose authority is the process's own argv.
    ///
    /// This is every non-remote-originated stdio session, and also the two
    /// surfaces whose authority is not a caller's to ask for at all: the TCP
    /// listener, which authenticates nobody and is hardcoded to `agent`, and
    /// the federated mux's client side, which is a caller here rather than a
    /// destination.
    pub fn local(authority: McpSessionAuthority) -> Self {
        Self {
            requested: authority.capabilities(),
            grant: None,
        }
    }

    /// A session capped by an already-resolved grant.
    ///
    /// `orbit mcp callers check` answers what a caller *would* get without
    /// serving a session, and must compute it the same way a served session
    /// does rather than restating the intersection.
    pub fn from_grant(authority: McpSessionAuthority, grant: ResolvedCallerGrant) -> Self {
        Self {
            requested: authority.capabilities(),
            grant: Some(grant),
        }
    }

    /// Resolve a stdio `orbit mcp serve` session against this destination.
    ///
    /// The caller's argv supplies the request; the callers file supplies the
    /// ceiling; the session gets the intersection. Because it is an
    /// intersection, the file can only lower a session — a caller granted
    /// `operator` that did not pass `--operator` still resolves to `agent`,
    /// so this opens no privilege path that argv alone did not already have.
    pub fn resolve(
        global_root: &Path,
        authority: McpSessionAuthority,
        caller_machine_id: &str,
    ) -> Result<Self, OrbitError> {
        let path = callers_path(global_root);
        let exists = path.exists();
        let file = load_callers(&path)?;
        if !exists {
            // The migration is a downgrade and will cut operator-over-SSH
            // flows on first upgrade. That is the intended direction, so it is
            // announced rather than silently applied.
            tracing::warn!(
                target: "orbit.mcp.callers",
                path = %path.display(),
                "no MCP callers file on this destination; remote sessions are served agent \
                 capabilities only — run `orbit mcp callers init` to declare callers"
            );
        }
        Ok(Self {
            requested: authority.capabilities(),
            grant: Some(file.resolve(caller_machine_id)),
        })
    }

    /// Whether the destination's callers file governs this session.
    pub fn is_granted(&self) -> bool {
        self.grant.is_some()
    }

    /// Effective capabilities for a call landing in `workspace_id`.
    pub fn effective_for(&self, workspace_id: Option<&str>) -> BTreeSet<McpCapability> {
        let Some(grant) = &self.grant else {
            return self.requested.clone();
        };
        let granted = grant.for_workspace(workspace_id);
        self.requested
            .intersection(&granted)
            .copied()
            .collect::<BTreeSet<_>>()
    }

    /// Effective capabilities anywhere a `workspaces` narrowing does not
    /// cover, so `orbit mcp callers check` can answer the per-workspace
    /// question a narrowed row actually poses instead of naming one scope.
    pub fn effective_outside_narrowing(&self) -> BTreeSet<McpCapability> {
        let Some(grant) = &self.grant else {
            return self.requested.clone();
        };
        self.requested
            .intersection(&grant.elsewhere)
            .copied()
            .collect()
    }

    /// The grant to record alongside the effective set for a call landing in
    /// `workspace_id`. `None` for a local session, which has no grant to
    /// distinguish from its own stamp.
    pub fn grant_for(&self, workspace_id: Option<&str>) -> Option<RemoteCallerGrant> {
        let grant = self.grant.as_ref()?;
        Some(RemoteCallerGrant {
            caller_machine_id: grant.caller_machine_id.clone(),
            granted_capabilities: grant.for_workspace(workspace_id),
            source: CALLERS_FILE_DISPLAY.to_string(),
        })
    }

    /// Stamp `context` with the capabilities and grant for a call landing in
    /// `workspace_id`.
    ///
    /// Called once at session establishment with the session's workspace, and
    /// again per call once the destination has resolved which registered
    /// workspace the call actually lands in — that resolution is the only
    /// point at which a `workspaces` narrowing can be evaluated.
    pub fn stamp(
        &self,
        context: &mut orbit_types::tool::ToolSessionContext,
        workspace_id: Option<&str>,
    ) {
        context.effective_capabilities = self.effective_for(workspace_id);
        context.remote_caller_grant = self.grant_for(workspace_id);
    }
}

/// One caller `orbit mcp callers init` seeds a row for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SeedCaller {
    pub machine_id: String,
    pub label: Option<String>,
}

/// Render a seed callers file granting every named caller `agent`.
///
/// `operator` is never written. The seeder exists to save an operator from
/// transcribing machine IDs, not to decide who may dispatch a workflow on this
/// machine — that has to stay a deliberate edit, or the file would re-create
/// the caller-authored grant it replaces.
pub fn render_callers_seed(callers: &[SeedCaller]) -> String {
    let mut out = String::from(
        "# Which callers this machine serves, and with what capabilities.\n\
         # Seeded by `orbit mcp callers init`, which grants agent only.\n\
         # Raising a row to operator is a deliberate hand edit.\n\
         \n\
         # Capabilities served to a caller that matches no row below.\n\
         # Permitted values: \"agent\" or \"deny\".\n\
         default = \"agent\"\n",
    );
    for caller in callers {
        out.push_str("\n[[callers]]\n");
        out.push_str(&format!("machine_id   = \"{}\"\n", caller.machine_id));
        if let Some(label) = &caller.label {
            out.push_str(&format!("label        = \"{label}\"\n"));
        }
        out.push_str("capabilities = [\"agent\"]\n");
    }
    out
}

/// Write a seed callers file, refusing to overwrite an existing one.
///
/// An existing file is an operator's statement about who may do what here;
/// re-running the seeder must never silently revoke an `operator` grant it is
/// forbidden to write back.
pub fn write_callers_seed(path: &Path, contents: &str) -> Result<(), OrbitError> {
    if path.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "MCP callers file '{}' already exists; edit it directly rather than re-seeding",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Io(format!("failed to create '{}': {error}", parent.display()))
        })?;
    }
    std::fs::write(path, contents).map_err(|error| {
        OrbitError::Io(format!(
            "failed to write MCP callers '{}': {error}",
            path.display()
        ))
    })
}
