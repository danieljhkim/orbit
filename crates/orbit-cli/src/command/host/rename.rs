use clap::Args;
use orbit_common::types::HostRecord;
use orbit_common::utility::fs::with_exclusive_file_lock;
use orbit_core::routines::host::HOST_TOML_FILE;
use orbit_core::routines::rename_current_host_identity;
use orbit_core::{HostRegistryService, OrbitError, OrbitRuntime, require_local_hub_identity};

use crate::command::Execute;

use super::command::resolve_machine_id;

#[derive(Args)]
#[command(about = "Rename a host, coordinating the local host.toml when it is this machine")]
pub struct HostRenameArgs {
    /// Current host name (or a tombstone alias resolving to the machine).
    current_name: String,
    /// New host name.
    new_name: String,
}

impl Execute for HostRenameArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let global_root = runtime.global_root();
        let local_hub = require_local_hub_identity(&global_root)?;
        let service = HostRegistryService::new(runtime.sqlite_store()?);
        service.require_configured_local_hub(&local_hub)?;
        let machine_id = resolve_machine_id(&service, &self.current_name)?;

        // Is the rename target this very machine? Only then does the local
        // host.toml participate. Renaming another machine never touches a local
        // file.
        let is_current_machine = local_hub.machine_id == machine_id;

        if is_current_machine {
            let lock_target = global_root.join(HOST_TOML_FILE);
            let record = coordinate_current_host_rename(
                &lock_target,
                &machine_id,
                &self.new_name,
                || service.validate_rename(&machine_id, &self.new_name),
                || rename_current_host_identity(&global_root, &self.new_name).map(|_| ()),
                || service.rename(&machine_id, &self.new_name),
                || service.host(&machine_id),
            )?;
            println!(
                "renamed this machine to '{}' (machine_id {}); local host.toml and the hub \
                 registry now agree",
                record.host_id, record.machine_id
            );
            Ok(())
        } else {
            // Registry-only rename for another machine; its local host.toml is
            // never pretended to be updated.
            let record = service.rename(&machine_id, &self.new_name)?;
            println!(
                "renamed host to '{}' (machine_id {}); this is a remote machine, so its local \
                 host.toml was not modified",
                record.host_id, record.machine_id
            );
            Ok(())
        }
    }
}

fn coordinate_current_host_rename<V, L, R, O>(
    lock_target: &std::path::Path,
    machine_id: &str,
    requested_host_id: &str,
    validate: V,
    rename_local: L,
    rename_registry: R,
    observe_registry: O,
) -> Result<HostRecord, OrbitError>
where
    V: FnOnce() -> Result<HostRecord, OrbitError>,
    L: FnOnce() -> Result<(), OrbitError>,
    R: FnOnce() -> Result<HostRecord, OrbitError>,
    O: FnOnce() -> Result<Option<HostRecord>, OrbitError>,
{
    // Serialize every cooperating current-machine rename across the entire
    // two-resource sequence. The store still revalidates inside its SQLite
    // transaction, while this sibling host.toml lock prevents two CLI
    // processes from each reporting success with opposing final identities.
    with_exclusive_file_lock(lock_target, "current host rename", || {
        let before = validate()?;
        if let Err(error) = rename_local() {
            return Err(OrbitError::InvalidInput(format!(
                "local host.toml rename did not report durable success: {error}. The hub \
                 registry rename was not attempted; if the error reports that the new local \
                 identity is readable, the local file and hub registry may now disagree and \
                 must be reconciled before further administration"
            )));
        }
        match rename_registry() {
            Ok(record) => {
                if record.machine_id != machine_id || record.host_id != requested_host_id {
                    return Err(OrbitError::Store(format!(
                        "registry rename returned unexpected identity '{}' ({}) instead of '{}' \
                         ({machine_id}); reconcile host.toml and the hub registry before further \
                         administration",
                        record.host_id, record.machine_id, requested_host_id
                    )));
                }
                Ok(record)
            }
            Err(error) => Err(classify_registry_rename_error(
                &before,
                requested_host_id,
                &error,
                observe_registry(),
            )),
        }
    })
}

fn classify_registry_rename_error(
    before: &HostRecord,
    requested_host_id: &str,
    rename_error: &OrbitError,
    observed: Result<Option<HostRecord>, OrbitError>,
) -> OrbitError {
    let detail = match observed {
        Ok(Some(host)) if host.host_id == requested_host_id => format!(
            "reopening the registry found the complete requested name '{}' for machine_id '{}'. \
             The registry commit may have succeeded despite the reported error, so its durability \
             is uncertain; the current local and registry views agree, but verify them before \
             further administration",
            host.host_id, host.machine_id
        ),
        Ok(Some(host)) if host.host_id == before.host_id => format!(
            "reopening the registry found the complete previous name '{}' for machine_id '{}'. \
             The registry rename is not visible, so local host.toml and the registry now disagree; \
             re-run `orbit host rename` once the hub is healthy to reconcile them",
            host.host_id, host.machine_id
        ),
        Ok(Some(host)) => format!(
            "reopening the registry found unexpected host_id '{}' for machine_id '{}', rather \
             than previous '{}' or requested '{}'. The registry outcome is unknown; reconcile \
             both identities before further administration",
            host.host_id, host.machine_id, before.host_id, requested_host_id
        ),
        Ok(None) => format!(
            "reopening the registry found no row for machine_id '{}'. The registry outcome is \
             unknown; reconcile both identities before further administration",
            before.machine_id
        ),
        Err(read_error) => format!(
            "reopening the registry to classify the outcome also failed: {read_error}. The \
             registry outcome is unknown; reconcile both identities before further administration"
        ),
    };
    OrbitError::InvalidInput(format!(
        "local host.toml was updated to '{requested_host_id}', but the hub registry rename \
         returned an error: {rename_error}. {detail}"
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use chrono::{TimeZone, Utc};
    use orbit_common::types::HostStatus;

    use super::*;

    fn host(host_id: &str) -> HostRecord {
        let now = Utc.with_ymd_and_hms(2026, 7, 18, 8, 0, 0).unwrap();
        HostRecord {
            machine_id: "hm_hub".to_string(),
            host_id: host_id.to_string(),
            labels: BTreeSet::new(),
            status: HostStatus::Active,
            registered_at: now,
            updated_at: now,
            retired_at: None,
            last_seen_at: Some(now),
        }
    }

    #[test]
    fn registry_rename_error_classifies_complete_new_and_complete_old_outcomes() {
        let before = host("old");
        let reported = OrbitError::Store("injected commit error".to_string());

        let committed =
            classify_registry_rename_error(&before, "new", &reported, Ok(Some(host("new"))))
                .to_string();
        assert!(committed.contains("commit may have succeeded"));
        assert!(committed.contains("durability is uncertain"));
        assert!(!committed.contains("did not commit"));

        let preserved =
            classify_registry_rename_error(&before, "new", &reported, Ok(Some(before.clone())))
                .to_string();
        assert!(preserved.contains("complete previous name 'old'"));
        assert!(preserved.contains("now disagree"));
        assert!(!preserved.contains("did not commit"));
    }

    #[test]
    fn concurrent_current_host_renames_serialize_the_file_and_registry_sequence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock_target = dir.path().join(HOST_TOML_FILE);
        let local_name = Arc::new(Mutex::new("old".to_string()));
        let registry_name = Arc::new(Mutex::new("old".to_string()));
        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let (first_local_tx, first_local_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let (second_validate_tx, second_validate_rx) = mpsc::channel();

        let first = {
            let lock_target = lock_target.clone();
            let local_name = Arc::clone(&local_name);
            let registry_name = Arc::clone(&registry_name);
            let events = Arc::clone(&events);
            std::thread::spawn(move || {
                coordinate_current_host_rename(
                    &lock_target,
                    "hm_hub",
                    "first",
                    || {
                        events.lock().expect("events").push("first:validate".into());
                        Ok(host(&registry_name.lock().expect("registry")))
                    },
                    || {
                        *local_name.lock().expect("local") = "first".to_string();
                        events.lock().expect("events").push("first:local".into());
                        first_local_tx.send(()).expect("signal first local");
                        release_first_rx.recv().expect("release first");
                        Ok(())
                    },
                    || {
                        *registry_name.lock().expect("registry") = "first".to_string();
                        events.lock().expect("events").push("first:registry".into());
                        Ok(host("first"))
                    },
                    || Ok(Some(host(&registry_name.lock().expect("registry")))),
                )
            })
        };
        first_local_rx.recv().expect("first reached local phase");

        let second = {
            let lock_target = lock_target.clone();
            let local_name = Arc::clone(&local_name);
            let registry_name = Arc::clone(&registry_name);
            let events = Arc::clone(&events);
            std::thread::spawn(move || {
                coordinate_current_host_rename(
                    &lock_target,
                    "hm_hub",
                    "second",
                    || {
                        second_validate_tx.send(()).expect("second entered");
                        events
                            .lock()
                            .expect("events")
                            .push("second:validate".into());
                        Ok(host(&registry_name.lock().expect("registry")))
                    },
                    || {
                        *local_name.lock().expect("local") = "second".to_string();
                        events.lock().expect("events").push("second:local".into());
                        Ok(())
                    },
                    || {
                        *registry_name.lock().expect("registry") = "second".to_string();
                        events
                            .lock()
                            .expect("events")
                            .push("second:registry".into());
                        Ok(host("second"))
                    },
                    || Ok(Some(host(&registry_name.lock().expect("registry")))),
                )
            })
        };

        let entered_while_first_was_paused = second_validate_rx
            .recv_timeout(Duration::from_millis(250))
            .is_ok();
        release_first_tx.send(()).expect("release first rename");
        first.join().expect("join first").expect("first rename");
        if !entered_while_first_was_paused {
            second_validate_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("second entered after first released the lock");
        }
        second.join().expect("join second").expect("second rename");

        assert!(
            !entered_while_first_was_paused,
            "second rename entered validation while first held the host identity lock"
        );
        assert_eq!(*local_name.lock().expect("local"), "second");
        assert_eq!(*registry_name.lock().expect("registry"), "second");
        assert_eq!(
            *events.lock().expect("events"),
            [
                "first:validate",
                "first:local",
                "first:registry",
                "second:validate",
                "second:local",
                "second:registry",
            ]
        );
    }
}
