mod model_pair_override {
    use super::super::super::executor_def::*;
    use crate::types::ExecutorResource;

    fn def_from_yaml(yaml: &str) -> ExecutorDef {
        let resource: ExecutorResource = serde_yaml::from_str(yaml).expect("parse executor yaml");
        ExecutorDef::from_resource_spec(
            resource.metadata.name,
            resource.spec.clone(),
            resource.spec.created_at,
            resource.spec.updated_at,
        )
    }

    #[test]
    fn roundtrips_model_pair_override_without_removed_models_key() {
        let def = def_from_yaml(
            r#"
schemaVersion: 2
kind: Executor
metadata:
  name: gemini
spec:
  executor_type: direct_agent
  command: gemini
  args:
    - -m
    - gemini-3.1-pro
  model_pair_override:
    strong: gemini-3.1-pro
    weak: gemini-3-flash
  model_flag: "-m"
"#,
        );

        assert_eq!(
            def.model_pair_override(),
            Some(&ModelPairOverride {
                strong: "gemini-3.1-pro".to_string(),
                weak: "gemini-3-flash".to_string(),
            })
        );
        assert_eq!(def.model_flag.as_deref(), Some("-m"));

        let serialized = serde_yaml::to_string(&def).expect("serialize executor def");
        assert!(
            serialized.contains("model_pair_override:"),
            "serialized executor def should use new key: {serialized}"
        );
        assert!(
            serialized.contains("model_flag: -m"),
            "serialized executor def should include model flag: {serialized}"
        );
        assert!(
            !serialized.contains("models:"),
            "serialized executor def should not use removed key: {serialized}"
        );
    }

    #[test]
    fn models_key_does_not_override_model_pair() {
        let def = def_from_yaml(
            r#"
schemaVersion: 2
kind: Executor
metadata:
  name: gemini
spec:
  executor_type: direct_agent
  command: gemini
  models:
    strong: gemini-3.1-pro
    weak: gemini-3-flash
"#,
        );

        assert!(
            def.model_pair_override().is_none(),
            "removed `models` key must not populate model_pair_override"
        );
    }
}
