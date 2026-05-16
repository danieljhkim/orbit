//! Shared friction taxonomy defaults.

/// Default friction tags and their human-readable glosses.
///
/// This list seeds `.orbit/frictions/tags.yaml` for new workspaces and keeps
/// tool-schema affordances aligned with the default validator vocabulary.
pub const DEFAULT_FRICTION_TAGS: &[(&str, &str)] = &[
    ("build", "make/fmt/lint friction"),
    ("docs", "Stale or missing CLAUDE.md or design docs"),
    ("lifecycle", "Task lifecycle confusion or transition issues"),
    ("naming", "Naming drift or duplicated sources of truth"),
    ("other", "Fallback"),
    ("policy", "fsProfile or sandboxing surprises"),
    (
        "skill-guidance",
        "Misleading or incorrect skill instructions",
    ),
    ("tooling", "Orbit tool/CLI/MCP failures"),
];

/// Return the default friction tag enum in schema-friendly literal form.
pub fn friction_tags_literal() -> String {
    DEFAULT_FRICTION_TAGS
        .iter()
        .map(|(tag, _description)| *tag)
        .collect::<Vec<_>>()
        .join(" | ")
}
