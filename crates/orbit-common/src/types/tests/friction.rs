mod frontmatter {
    use super::super::super::friction::*;

    #[test]
    fn friction_frontmatter_without_resolved_by_task_deserializes() {
        let yaml = r#"
id: F2026-05-007
model: codex
created_at: 2026-05-17T04:05:00Z
status: open
tags:
  - tooling
during_task: ORB-00093
"#;

        let frontmatter: FrictionFrontmatter = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(frontmatter.id, "F2026-05-007");
        assert_eq!(frontmatter.resolved_by_task, None);

        let serialized = serde_yaml::to_string(&frontmatter).unwrap();
        let round_trip: FrictionFrontmatter = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(round_trip.resolved_by_task, None);
    }
}
