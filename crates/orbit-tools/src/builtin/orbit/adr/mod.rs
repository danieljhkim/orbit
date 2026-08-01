pub mod add;
pub mod list;
pub mod restore;
pub mod show;
pub mod supersede;
pub mod update;

use orbit_common::types::ToolParam;

pub(super) fn create_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "title".to_string(),
            description: "ADR title (short noun phrase).".to_string(),
            param_type: "string".to_string(),
            required: true,
        },
        ToolParam {
            name: "body".to_string(),
            description:
                "ADR body as markdown. Must include Context / Decision / Consequences sections per the ADR template; at least one consequences bullet must be a labeled `Cost:` line."
                    .to_string(),
            param_type: "string".to_string(),
            required: true,
        },
        ToolParam {
            name: "owner".to_string(),
            description:
                "Agent identity that owns the ADR (e.g. `claude`, `codex`). Defaults to the calling actor."
                    .to_string(),
            param_type: "string".to_string(),
            required: false,
        },
        ToolParam {
            name: "related_features".to_string(),
            description:
                "Feature folder names this decision touches. Accepts a string or array of strings."
                    .to_string(),
            param_type: "string_list".to_string(),
            required: false,
        },
        ToolParam {
            name: "related_tasks".to_string(),
            description:
                "Orbit task IDs that proposed or shipped the decision. May be empty at creation per ADR-008. Accepts a string or array of strings."
                    .to_string(),
            param_type: "string_list".to_string(),
            required: false,
        },
        ToolParam {
            name: "tags".to_string(),
            description: "Free-form ADR labels. Defaults to an empty list when omitted."
                .to_string(),
            param_type: "string_list".to_string(),
            required: false,
        },
        ToolParam {
            name: "paths".to_string(),
            description:
                "Repo-relative glob patterns for code or docs areas constrained by this ADR. Defaults to an empty list when omitted."
                    .to_string(),
            param_type: "string_list".to_string(),
            required: false,
        },
    ]
}
