//! Fake destinations: a scripted probe stands in for every SSH session, so the
//! mux's projection and failure handling are exercised without a live host.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{TimeZone, Utc};
use orbit_common::OrbitError;
use orbit_types::workspace::{Workspace, WorkspaceStatus};

use super::super::config::Destination;
use super::super::probe::{DestinationProbe, DestinationSnapshot};

pub(super) const OWNER_MACHINE: &str = "hm_owner";
pub(super) const REPLICA_MACHINE: &str = "hm_replica";

pub(super) fn destination(ssh: &str, machine_id: &str) -> Destination {
    Destination {
        ssh: ssh.to_string(),
        machine_id: machine_id.to_string(),
    }
}

pub(super) fn workspace(id: &str, owner_machine_id: Option<&str>) -> Workspace {
    // A fixed timestamp keeps descriptor assertions stable.
    let at = Utc
        .with_ymd_and_hms(2026, 8, 23, 0, 0, 0)
        .single()
        .expect("fixture timestamp");
    Workspace {
        id: id.to_string(),
        name: id.trim_start_matches("ws_").to_string(),
        owner_machine_id: owner_machine_id.map(ToOwned::to_owned),
        git_remote: None,
        ship_mode: None,
        base_branch: "main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: at,
        updated_at: at,
    }
}

/// How many times the mux actually reached out.
#[derive(Clone)]
pub(super) struct ProbeCallCounter(Arc<AtomicUsize>);

impl ProbeCallCounter {
    pub(super) fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// A probe with one canned outcome per destination `machine_id`.
pub(super) struct ScriptedProbe {
    outcomes: HashMap<String, Result<DestinationSnapshot, OrbitError>>,
    calls: Arc<AtomicUsize>,
}

impl ScriptedProbe {
    pub(super) fn new() -> Self {
        Self {
            outcomes: HashMap::new(),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn answering(mut self, machine_id: &str, snapshot: DestinationSnapshot) -> Self {
        self.outcomes.insert(machine_id.to_string(), Ok(snapshot));
        self
    }

    pub(super) fn refusing(mut self, machine_id: &str, error: OrbitError) -> Self {
        self.outcomes.insert(machine_id.to_string(), Err(error));
        self
    }

    pub(super) fn call_counter(&self) -> ProbeCallCounter {
        ProbeCallCounter(Arc::clone(&self.calls))
    }
}

impl DestinationProbe for ScriptedProbe {
    fn probe(&self, destination: &Destination) -> Result<DestinationSnapshot, OrbitError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.outcomes.get(&destination.machine_id) {
            Some(Ok(snapshot)) => Ok(snapshot.clone()),
            // `OrbitError` is not `Clone`, so a refusal is restated rather than
            // copied; the variant is what the mux branches on.
            Some(Err(error)) => Err(OrbitError::UnreachableDestination(error.to_string())),
            None => Err(OrbitError::UnreachableDestination(format!(
                "{}: no scripted outcome",
                destination.machine_id
            ))),
        }
    }
}
