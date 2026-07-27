use chrono::{Duration, TimeZone, Utc};

use super::super::{
    Crew, CrewRoleAssignment, ExecutionProfileCrewV1, ExecutionProfileShipV1, ExecutionProfileV1,
};
use crate::types::activity_job::Backend;

fn profile() -> ExecutionProfileV1 {
    let crews = vec![ExecutionProfileCrewV1 {
        name: "alpha".to_string(),
        provider: "codex".to_string(),
        model: "gpt-test".to_string(),
        backend: "cli".to_string(),
        description: None,
        tags: vec!["fast".to_string(), "review".to_string()],
    }];
    let mut profile = ExecutionProfileV1 {
        schema_version: 1,
        workspace_id: "ws_alpha".to_string(),
        owner_machine_id: "hm_alpha".to_string(),
        observed_at: Utc
            .with_ymd_and_hms(2026, 7, 18, 8, 0, 0)
            .single()
            .expect("timestamp"),
        config_digest: String::new(),
        default_crew: "alpha".to_string(),
        crews,
        ship: ExecutionProfileShipV1 {
            mode: "pr".to_string(),
            base_branch: "agent-main".to_string(),
            ship_closure_digest: "a".repeat(64),
        },
    };
    profile.config_digest = profile.compute_config_digest().expect("digest");
    profile
}

#[test]
fn crew_normalization_canonicalizes_alias_metadata_and_auto_backend() {
    let crew = Crew {
        name: " alpha ".to_string(),
        assignment: CrewRoleAssignment {
            provider: " OpenAI ".to_string(),
            model: " gpt-test ".to_string(),
            backend: "auto".to_string(),
        },
        description: Some("  Fast implementation  ".to_string()),
        tags: vec![
            " zeta ".to_string(),
            String::new(),
            "alpha".to_string(),
            "alpha".to_string(),
        ],
    };

    let normalized =
        ExecutionProfileCrewV1::from_crew(&crew, Backend::Cli).expect("crew normalizes");
    assert_eq!(normalized.name, "alpha");
    assert_eq!(normalized.provider, "codex");
    assert_eq!(normalized.model, "gpt-test");
    assert_eq!(normalized.backend, "cli");
    assert_eq!(
        normalized.description.as_deref(),
        Some("Fast implementation")
    );
    assert_eq!(normalized.tags, vec!["alpha", "zeta"]);
}

#[test]
fn config_digest_excludes_identity_observation_closure_and_hub_fields() {
    let original = profile();
    let mut changed_excluded = original.clone();
    changed_excluded.workspace_id = "ws_other".to_string();
    changed_excluded.owner_machine_id = "hm_other".to_string();
    changed_excluded.observed_at += Duration::hours(1);
    changed_excluded.ship.ship_closure_digest = "b".repeat(64);
    assert_eq!(
        original.compute_config_digest().expect("original digest"),
        changed_excluded
            .compute_config_digest()
            .expect("excluded digest")
    );

    for mutate in [
        |profile: &mut ExecutionProfileV1| profile.crews[0].model.push_str("-new"),
        |profile: &mut ExecutionProfileV1| profile.ship.mode = "local".to_string(),
        |profile: &mut ExecutionProfileV1| profile.ship.base_branch = "main".to_string(),
    ] {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert_ne!(
            original.compute_config_digest().expect("original digest"),
            changed.compute_config_digest().expect("changed digest")
        );
    }
}

#[test]
fn validation_rejects_lossy_or_ambiguous_profile_shapes() {
    let mut unsorted = profile();
    unsorted.crews.push(ExecutionProfileCrewV1 {
        name: "aardvark".to_string(),
        provider: "codex".to_string(),
        model: "gpt-test".to_string(),
        backend: "cli".to_string(),
        description: None,
        tags: Vec::new(),
    });
    assert!(unsorted.validate().is_err());

    let mut auto = profile();
    auto.crews[0].backend = "auto".to_string();
    assert!(auto.validate().is_err());

    let mut alias = profile();
    alias.crews[0].provider = "openai".to_string();
    assert!(alias.validate().is_err());

    let mut unknown_provider = profile();
    unknown_provider.crews[0].provider = "ambiguous".to_string();
    assert!(unknown_provider.validate().is_err());

    let mut unknown_backend = profile();
    unknown_backend.crews[0].backend = "ambiguous".to_string();
    assert!(unknown_backend.validate().is_err());

    let mut missing_default = profile();
    missing_default.default_crew = "missing".to_string();
    missing_default.config_digest = missing_default.compute_config_digest().expect("digest");
    assert!(missing_default.validate().is_err());
}
