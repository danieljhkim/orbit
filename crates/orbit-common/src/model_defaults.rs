//! Single source of truth for Orbit's default model names.
//!
//! Default model names used to be hardcoded as version-pinned string literals
//! scattered across crates, and had drifted out of sync — the Claude default
//! appeared as `claude-opus-4-7` in one place, `claude-sonnet-4-6` in another,
//! and `claude-sonnet-4-5` in a third. This module centralizes the production
//! defaults so a single edit updates every code path and the drift cannot
//! recur.
//!
//! ## Asset ↔ const seam
//!
//! YAML/TOML assets (executor definitions under `assets/executors/*.yaml`,
//! seeded `config.toml`) cannot reference a Rust const. For those, "single
//! source of truth" means using the provider alias directly in the asset and
//! keeping this module authoritative for production Rust paths. The executor
//! asset ↔ const agreement is guarded by a test in orbit-core's executor
//! command module. See ADR-0211 for the de-hardcode + centralization decision.
//!
//! ## Aliases vs. version pins
//!
//! The Claude CLI accepts the unversioned `opus`/`sonnet` aliases and resolves
//! them to the current flagship, so the CLI defaults never drift. The Anthropic
//! **HTTP Messages API** rejects bare aliases and requires a fully-qualified
//! model id, so [`ANTHROPIC_HTTP_DEFAULT_MODEL`] stays version pinned. codex,
//! gemini, and grok use provider-specific version pins because no stable
//! unversioned aliases are available for those CLIs.

/// Default "strong" Claude model: the unversioned `opus` CLI alias.
pub const CLAUDE_DEFAULT_STRONG: &str = "opus";

/// Default "weak" Claude model: the unversioned `sonnet` CLI alias.
pub const CLAUDE_DEFAULT_WEAK: &str = "sonnet";

/// Claude CLI alias used by the standard Fable crew.
pub const CLAUDE_FABLE_MODEL: &str = "fable";

/// Default model for the Anthropic **HTTP Messages API** path.
///
/// Unlike the CLI, the Messages API requires a fully-qualified model id and
/// rejects bare aliases like `opus`/`sonnet`, so this default stays version
/// pinned. Bump it in lockstep with Anthropic's published API model ids.
pub const ANTHROPIC_HTTP_DEFAULT_MODEL: &str = "claude-sonnet-4-5";

/// Codex model used by the standard Sol crew.
pub const CODEX_SOL_MODEL: &str = "gpt-5.6-sol";

/// Codex model used by the standard Terra crew and provider default.
pub const CODEX_TERRA_MODEL: &str = "gpt-5.6-terra";

/// Codex model used by the standard Luna crew.
pub const CODEX_LUNA_MODEL: &str = "gpt-5.6-luna";

/// Default codex model (Terra remains the provider and QA default).
pub const CODEX_DEFAULT_MODEL: &str = CODEX_TERRA_MODEL;

/// Default codex "weak" model used by the executor model pair.
pub const CODEX_DEFAULT_WEAK: &str = "gpt-5.4-mini";

/// Default gemini model for the provider-default map.
pub const GEMINI_DEFAULT_MODEL: &str = "gemini-3-pro";

/// Default gemini model seeded into crew roles (the `pro` CLI alias).
pub const GEMINI_CREW_MODEL: &str = "pro";

/// Default gemini "strong" model used by the executor model pair.
pub const GEMINI_PAIR_STRONG: &str = "gemini-3.1-pro";

/// Default gemini "weak" model used by the executor model pair.
pub const GEMINI_PAIR_WEAK: &str = "gemini-3-flash";

/// Default grok model (single value used everywhere grok defaults appear).
pub const GROK_DEFAULT_MODEL: &str = "grok-build";

/// Cheap Claude model used by the orbit-agent HTTP examples.
///
/// Version pinned like [`ANTHROPIC_HTTP_DEFAULT_MODEL`] because the examples
/// hit the Anthropic Messages API directly, which rejects bare aliases.
pub const ANTHROPIC_EXAMPLE_MODEL: &str = "claude-haiku-4-5-20251001";

/// Provider → default model used to seed prompt/duel defaults.
///
/// Mirrors the historical `agent_detect::default_model_for` map; `claude` now
/// resolves to the unversioned [`CLAUDE_DEFAULT_STRONG`] alias. codex/gemini/
/// grok use their provider-specific defaults.
pub fn default_model_for_provider(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some(CLAUDE_DEFAULT_STRONG),
        "codex" => Some(CODEX_DEFAULT_MODEL),
        "gemini" => Some(GEMINI_DEFAULT_MODEL),
        "grok" => Some(GROK_DEFAULT_MODEL),
        _ => None,
    }
}
