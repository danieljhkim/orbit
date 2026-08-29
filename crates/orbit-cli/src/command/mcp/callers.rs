//! `orbit mcp callers` — read and seed this machine's caller authorization.
//!
//! The file is an operator artifact, so these subcommands are deliberately
//! read-mostly: `list` and `check` answer "what would this destination serve",
//! and `init` transcribes machine IDs the operator already has. Granting
//! `operator` stays a hand edit [ORB-11052].

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use clap::{Args, Subcommand};
use orbit_core::OrbitError;
use orbit_core::runtime::resolve_global_root;
use orbit_mcp::{
    DefaultGrant, McpSessionAuthority, SeedCaller, SessionCapabilityPolicy, federated,
};
use orbit_types::tool::McpCapability;

use crate::command::{CommandOut, CommandOutput};

#[derive(Args)]
#[command(
    about = "Inspect and seed this machine's MCP caller authorization",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct CallersArgs {
    #[command(subcommand)]
    pub command: CallersSubcommand,
}

#[derive(Subcommand)]
pub enum CallersSubcommand {
    /// Print the callers this machine serves and the default it applies to
    /// everyone else
    List(CallersListArgs),
    /// Print what a session from one caller would resolve to, without serving
    /// one
    Check(CallersCheckArgs),
    /// Seed a callers file from the machines this one already knows about
    ///
    /// Every seeded row is granted the agent capability. Operator is never
    /// written: which callers may dispatch work on this machine is a decision
    /// to make deliberately, one row at a time.
    Init(CallersInitArgs),
}

#[derive(Args)]
#[command(about = "Print the callers this machine serves")]
pub struct CallersListArgs;

#[derive(Args)]
#[command(about = "Print what a session from one caller would resolve to")]
pub struct CallersCheckArgs {
    /// The calling machine's stable identity, as it would be forwarded.
    #[arg(value_name = "MACHINE_ID")]
    pub machine_id: String,
}

#[derive(Args)]
#[command(about = "Seed a callers file granting agent to known machines")]
pub struct CallersInitArgs;

impl CallersArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> CommandOut {
        if root_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "orbit mcp callers does not accept a workspace root override; the callers file is \
                 machine-global"
                    .to_string(),
            ));
        }
        let global_root = resolve_global_root()?;
        let path = orbit_mcp::callers_path(&global_root);
        match self.command {
            CallersSubcommand::List(_) => list(&path),
            CallersSubcommand::Check(args) => check(&path, &args.machine_id),
            CallersSubcommand::Init(_) => init(&global_root, &path),
        }
    }
}

fn list(path: &Path) -> CommandOut {
    let file = orbit_mcp::load_callers(path)?;
    println!("callers file: {}", path.display());
    if !path.exists() {
        println!(
            "  (absent — remote-originated sessions are served agent capabilities only; run \
             `orbit mcp callers init`)"
        );
    }
    println!("default: {}", default_label(file.default));
    if file.callers.is_empty() {
        println!("callers: none");
        return Ok(CommandOutput::Silent);
    }
    println!("callers:");
    for row in &file.callers {
        let label = row
            .label
            .as_deref()
            .map(|label| format!(" ({label})"))
            .unwrap_or_default();
        println!(
            "  {machine_id}{label}: [{capabilities}]",
            machine_id = row.machine_id,
            capabilities = row.capabilities.join(", "),
        );
        if let Some(workspaces) = &row.workspaces {
            println!("    workspaces: {}", workspaces.join(", "));
        }
        if let Some(fingerprint) = &row.ssh_key_fingerprint {
            // Parsed but not yet verified: nothing observes the authenticating
            // key until the forced-command path exists [ORB-11053].
            println!("    ssh_key_fingerprint: {fingerprint} (recorded, not yet verified)");
        }
    }
    Ok(CommandOutput::Silent)
}

fn check(path: &Path, machine_id: &str) -> CommandOut {
    let file = orbit_mcp::load_callers(path)?;
    let grant = file.resolve(machine_id);
    println!("callers file: {}", path.display());
    println!("caller: {machine_id}");
    println!(
        "matched: {}",
        if grant.matched {
            "a row"
        } else {
            "no row — the file default applies"
        }
    );
    println!("granted: [{}]", capability_list(&grant.granted));
    if let Some(workspaces) = &grant.workspaces {
        println!(
            "  on workspaces: {}",
            workspaces.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        println!(
            "  elsewhere on this machine: [{}]",
            capability_list(&grant.elsewhere)
        );
    }
    // Both requests, because the grant is a ceiling and the caller's argv is
    // the other half of the intersection: printing only one would read as the
    // answer to a question the caller did not ask.
    println!("a remote-originated session from this caller would hold:");
    for (label, authority) in [
        ("orbit mcp serve", McpSessionAuthority::Agent),
        ("orbit mcp serve --operator", McpSessionAuthority::Operator),
    ] {
        let policy = SessionCapabilityPolicy::from_grant(authority, file.resolve(machine_id));
        // A narrowing makes "what would this session hold" a per-workspace
        // question, so answering it with one set would be a half-truth.
        match &grant.workspaces {
            None => println!(
                "  {label}: [{}]",
                capability_list(&policy.effective_for(None))
            ),
            Some(workspaces) => {
                for workspace in workspaces {
                    println!(
                        "  {label} on {workspace}: [{}]",
                        capability_list(&policy.effective_for(Some(workspace)))
                    );
                }
                println!(
                    "  {label} elsewhere: [{}]",
                    capability_list(&policy.effective_for(Some("")))
                );
            }
        }
    }
    Ok(CommandOutput::Silent)
}

fn init(global_root: &Path, path: &Path) -> CommandOut {
    let callers = known_callers(global_root)?;
    if callers.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "no other machines are known to this one, so there is nothing to seed; write '{}' by \
             hand from the caller's `machine_id`",
            path.display()
        )));
    }
    let contents = orbit_mcp::render_callers_seed(&callers);
    orbit_mcp::write_callers_seed(path, &contents)?;
    println!("wrote {}", path.display());
    for caller in &callers {
        println!("  {} granted [agent]", caller.machine_id);
    }
    println!(
        "Grant operator by editing the file; `orbit mcp callers init` never writes that \
         capability."
    );
    Ok(CommandOutput::Silent)
}

/// Machines this one already names: the owners of its registered workspaces
/// and the destinations it is configured to call.
///
/// Neither list is a fleet inventory, and neither is an authorization input —
/// they are only where an operator's machine IDs are already written down, so
/// seeding does not become a transcription exercise. The accepting machine is
/// excluded: a row for itself would authorize nothing it does not already
/// resolve locally.
fn known_callers(global_root: &Path) -> Result<Vec<SeedCaller>, OrbitError> {
    // Absent host identity means this machine has no stable `machine_id`, so
    // there is nothing to exclude — and nothing that could collide either.
    let local_machine_id = match orbit_registry::inspect_host_identity(global_root)? {
        orbit_registry::HostIdentityState::Present(identity) => Some(identity.machine_id),
        _ => None,
    };
    let registry_path = orbit_registry::workspace_registry::registry_path_for(global_root);
    let registry = orbit_registry::workspace_registry::load_registry_from(&registry_path)?;
    let mut labels: BTreeMap<String, Option<String>> = BTreeMap::new();
    for workspace in &registry.workspaces {
        if let Some(owner) = &workspace.owner_machine_id {
            labels
                .entry(owner.clone())
                .or_insert_with(|| registry.owner_host_ids.get(owner).cloned());
        }
    }
    let destinations =
        federated::load_destinations(&federated::destinations_path(global_root))?.destinations;
    for destination in destinations {
        labels
            .entry(destination.machine_id.clone())
            .or_insert(Some(destination.ssh));
    }
    if let Some(local_machine_id) = &local_machine_id {
        labels.remove(local_machine_id);
    }
    Ok(labels
        .into_iter()
        .map(|(machine_id, label)| SeedCaller { machine_id, label })
        .collect())
}

fn default_label(default: DefaultGrant) -> &'static str {
    match default {
        DefaultGrant::Agent => "agent",
        DefaultGrant::Deny => "deny",
    }
}

fn capability_list(capabilities: &BTreeSet<McpCapability>) -> String {
    if capabilities.is_empty() {
        return "none".to_string();
    }
    capabilities
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "tests/callers.rs"]
mod tests;
