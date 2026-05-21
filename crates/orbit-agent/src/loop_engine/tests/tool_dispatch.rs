#![allow(missing_docs)]

mod schema {
    #![allow(missing_docs)]

    use super::super::super::tool_dispatch::*;

    use orbit_common::types::{ToolParam, ToolSchema};
    use orbit_tools::ToolRegistry;

    fn param_with_type(name: &str, param_type: &str) -> ToolParam {
        ToolParam {
            name: name.to_string(),
            description: String::new(),
            param_type: param_type.to_string(),
            required: false,
        }
    }

    fn param(name: &str) -> ToolParam {
        param_with_type(name, "string")
    }

    #[test]
    fn task_add_schema_excludes_legacy_friction_enums() {
        let add_schema = ToolSchema {
            name: "orbit.task.add".to_string(),
            description: String::new(),
            parameters: vec![param("type"), param("status")],
            builtin: true,
        };
        let add_spec = schema_to_tool_spec(&add_schema);
        let add_properties = add_spec.input_schema["properties"]
            .as_object()
            .expect("properties");
        assert!(
            !add_properties["type"]["enum"]
                .as_array()
                .expect("type enum")
                .iter()
                .any(|value| value == "friction")
        );
        assert!(
            !add_properties["status"]["enum"]
                .as_array()
                .expect("status enum")
                .iter()
                .any(|value| value == "friction")
        );
    }

    #[test]
    fn task_update_schema_preserves_legacy_friction_status_enum() {
        let update_schema = ToolSchema {
            name: "orbit.task.update".to_string(),
            description: String::new(),
            parameters: vec![param("status")],
            builtin: true,
        };
        let update_spec = schema_to_tool_spec(&update_schema);
        assert!(
            update_spec.input_schema["properties"]["status"]["enum"]
                .as_array()
                .expect("update status enum")
                .iter()
                .any(|value| value == "friction")
        );
    }

    #[test]
    fn task_tool_specs_advertise_dependencies_as_string_list() {
        for tool_name in ["orbit.task.add", "orbit.task.update"] {
            let schema = ToolSchema {
                name: tool_name.to_string(),
                description: String::new(),
                parameters: vec![param_with_type("dependencies", "string_list")],
                builtin: true,
            };
            let spec = schema_to_tool_spec(&schema);
            let any_of = spec.input_schema["properties"]["dependencies"]["anyOf"]
                .as_array()
                .expect("string-list union");

            assert!(
                any_of.iter().any(|schema| {
                    schema["type"] == "array" && schema["items"]["type"] == "string"
                }),
                "{tool_name} dependencies must accept an array of strings"
            );
            assert!(
                any_of.iter().any(|schema| schema["type"] == "string"),
                "{tool_name} dependencies must accept a string"
            );
        }
    }

    #[test]
    fn build_tool_specs_expands_wildcards_to_registered_tools_only() {
        // ORB-00202: `orbit.task.search` was hard-removed in phase 2, so it
        // is the natural "known-unregistered" name to assert the wildcard
        // skips. No prior `unregister` call is needed.
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let specs = build_tool_specs(&registry, &["orbit.task.*".to_string()]);
        let names = specs.into_iter().map(|spec| spec.name).collect::<Vec<_>>();

        assert!(names.iter().any(|name| name == "orbit.task.show"));
        assert!(!names.iter().any(|name| name == "orbit.task.search"));
        assert!(!names.iter().any(|name| name == "orbit.task.*"));
    }

    #[test]
    fn build_tool_specs_does_not_expand_unvalidated_wildcard_roots() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let specs = build_tool_specs(&registry, &["orbit.*".to_string()]);

        assert!(specs.is_empty());
    }
}
