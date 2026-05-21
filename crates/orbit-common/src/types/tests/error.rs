mod serialization {
    use super::super::super::error::{NotFoundKind, OrbitError};

    #[test]
    fn orbit_not_found_error_serializes_with_typed_kind() {
        let error = OrbitError::NotFound {
            kind: NotFoundKind::Task,
            id: "ORB-00001".to_string(),
        };

        let value = serde_json::to_value(error).expect("serialize orbit error");

        assert_eq!(
            value,
            serde_json::json!({
                "NotFound": {
                    "kind": "task",
                    "id": "ORB-00001"
                }
            })
        );
    }
}
