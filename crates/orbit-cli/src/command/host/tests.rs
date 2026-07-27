use std::collections::BTreeSet;

use chrono::Utc;
use orbit_common::types::{HostStatus, RegistryHostV1};

use super::list::format_host_list;

fn host(machine_id: &str, host_id: &str, labels: &[&str]) -> RegistryHostV1 {
    let now = Utc::now();
    RegistryHostV1 {
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        labels: labels
            .iter()
            .map(|label| (*label).to_string())
            .collect::<BTreeSet<_>>(),
        status: HostStatus::Active,
        registered_at: now,
        updated_at: now,
        retired_at: None,
        last_seen_at: None,
        aliases: Vec::new(),
        presence: Vec::new(),
    }
}

#[test]
fn format_host_list_handles_empty() {
    assert_eq!(format_host_list(&[], None), "no hosts registered\n");
}

#[test]
fn format_host_list_marks_hub_and_renders_labels() {
    let hosts = vec![
        host("hm_hub", "hub", &["codex"]),
        host("hm_spoke", "spoke", &[]),
    ];
    let rendered = format_host_list(&hosts, Some("hm_hub"));
    let hub_line = rendered
        .lines()
        .find(|line| line.contains("hm_hub"))
        .expect("hub line");
    assert!(hub_line.contains("yes"), "hub not flagged: {hub_line}");
    assert!(hub_line.contains("codex"), "labels missing: {hub_line}");
    let spoke_line = rendered
        .lines()
        .find(|line| line.contains("hm_spoke"))
        .expect("spoke line");
    // The spoke is not the hub and has no labels.
    assert!(
        !spoke_line.contains("yes"),
        "spoke wrongly flagged: {spoke_line}"
    );
}
