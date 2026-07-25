//! Model-string identity: telling an exact provider model string apart from
//! the unversioned CLI alias a crew was dispatched with (ORB-10354).
//!
//! Orbit's crew config deliberately dispatches Claude through unversioned CLI
//! aliases (`opus`, `sonnet`, `fable`) so the CLI defaults never drift, and
//! gemini through the `pro` alias. Those alias strings used to flow straight
//! into `invocations.model`, where the price table — keyed by exact model
//! string (ADR-0245) — could never match them: alias rows derived no cost and
//! sat in per-model aggregates as if they were their own model.
//!
//! This module is the single place that maps an alias to the exact string it
//! currently resolves to, so the ingest path can record a resolved version
//! string in the `model` column and keep the alias as provenance metadata.
//!
//! ## Why a local table, and how it goes stale
//!
//! The authoritative resolution lives with the provider: the Claude CLI picks
//! the current flagship behind `opus` at dispatch time, and it reports the
//! exact string it used in its result JSON (`modelUsage` keys). Orbit's ingest
//! path does not yet parse that field, so the only resolution available at
//! insert time is this table. When a provider promotes a new flagship behind an
//! existing alias, this table is stale until it is updated — the entry must be
//! bumped in the same change as the matching `model_prices.yaml` row.
//! [`crate::types::pricing`] documents the price side of the same seam.
//!
//! An alias whose resolution is genuinely ambiguous carries
//! `resolves_to: None` and classifies as [`ModelIdentity::UnresolvedAlias`]:
//! recorded as metadata, never guessed into the `model` column.

use crate::model_defaults::{
    CLAUDE_DEFAULT_STRONG, CLAUDE_DEFAULT_WEAK, CLAUDE_FABLE_MODEL, GEMINI_CREW_MODEL,
};

/// Exact model string the Claude CLI's `opus` alias currently resolves to.
///
/// Telemetry attribution only — never pass this to the CLI in place of the
/// alias, which is what keeps dispatch on the current flagship.
pub const CLAUDE_OPUS_ALIAS_TARGET: &str = "claude-opus-5";

/// Exact model string the Claude CLI's `sonnet` alias currently resolves to.
pub const CLAUDE_SONNET_ALIAS_TARGET: &str = "claude-sonnet-5";

/// Exact model string the Claude CLI's `fable` alias currently resolves to.
pub const CLAUDE_FABLE_ALIAS_TARGET: &str = "claude-fable-5";

/// One unversioned alias an Orbit crew can be dispatched with, scoped to the
/// agent family that owns it. `resolves_to` is `None` when Orbit cannot name
/// the exact string the alias lands on.
struct ModelAliasEntry {
    family: &'static str,
    alias: &'static str,
    resolves_to: Option<&'static str>,
}

/// Every unversioned model alias Orbit itself dispatches with, from
/// `[crews.*].model` in `config.toml` and the executor `model_pair_override`
/// assets. Keep this list to strings that actually reach the ingest path (the
/// same rule `model_prices.yaml` follows, per L-0107) — a speculative entry
/// cannot be validated against real data.
const MODEL_ALIASES: &[ModelAliasEntry] = &[
    ModelAliasEntry {
        family: "claude",
        alias: CLAUDE_DEFAULT_STRONG,
        resolves_to: Some(CLAUDE_OPUS_ALIAS_TARGET),
    },
    ModelAliasEntry {
        family: "claude",
        alias: CLAUDE_DEFAULT_WEAK,
        resolves_to: Some(CLAUDE_SONNET_ALIAS_TARGET),
    },
    ModelAliasEntry {
        family: "claude",
        alias: CLAUDE_FABLE_MODEL,
        resolves_to: Some(CLAUDE_FABLE_ALIAS_TARGET),
    },
    // The gemini crew dispatches `pro`, but Orbit pins two different exact
    // gemini pro strings for different purposes (`GEMINI_DEFAULT_MODEL` =
    // gemini-3-pro, `GEMINI_PAIR_STRONG` = gemini-3.1-pro), so naming one of
    // them here would fabricate the resolution. Recorded as metadata until the
    // ingest path can read the provider-reported string.
    ModelAliasEntry {
        family: "gemini",
        alias: GEMINI_CREW_MODEL,
        resolves_to: None,
    },
];

/// What an invocation's requested model string turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelIdentity {
    /// An exact provider model string, safe to key the price table with.
    Exact(String),
    /// A known alias, plus the exact string it resolves to.
    ResolvedAlias { model: String, alias: String },
    /// A known alias Orbit cannot resolve locally. Carries no model string, so
    /// callers must not put it where an exact model string belongs.
    UnresolvedAlias { alias: String },
}

impl ModelIdentity {
    /// The exact model string, when one is known. `None` for
    /// [`Self::UnresolvedAlias`].
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Exact(model) | Self::ResolvedAlias { model, .. } => Some(model.as_str()),
            Self::UnresolvedAlias { .. } => None,
        }
    }

    /// The alias the invocation was dispatched with, when it was dispatched
    /// through one. `None` for [`Self::Exact`].
    pub fn alias(&self) -> Option<&str> {
        match self {
            Self::Exact(_) => None,
            Self::ResolvedAlias { alias, .. } | Self::UnresolvedAlias { alias } => {
                Some(alias.as_str())
            }
        }
    }
}

/// Classify the model string an invocation was dispatched with.
///
/// `agent_family` disambiguates aliases across providers; pass `None` when the
/// family is unknown and the alias will still resolve as long as exactly one
/// family claims it. Returns `None` for an empty or whitespace-only `raw`.
/// A string absent from [`MODEL_ALIASES`] is treated as exact — an alias Orbit
/// has never dispatched with reaches the store as-is and surfaces through the
/// unpriced-model scan rather than being silently reshaped here.
pub fn classify_model_string(agent_family: Option<&str>, raw: &str) -> Option<ModelIdentity> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let lowered = raw.to_ascii_lowercase();
    let claimed: Vec<&ModelAliasEntry> = MODEL_ALIASES
        .iter()
        .filter(|entry| entry.alias == lowered)
        .collect();
    if claimed.is_empty() {
        return Some(ModelIdentity::Exact(raw.to_string()));
    }

    let family = agent_family
        .map(|family| family.trim().to_ascii_lowercase())
        .filter(|family| !family.is_empty());
    let resolved = match family {
        Some(family) => claimed
            .iter()
            .find(|entry| entry.family == family)
            .and_then(|entry| entry.resolves_to),
        // No family to disambiguate with: only resolve when the alias is
        // unambiguous across providers.
        None => claimed
            .first()
            .filter(|_| claimed.len() == 1)
            .and_then(|entry| entry.resolves_to),
    };

    // A string some family claims as an alias is never recorded as an exact
    // model, even when this family doesn't claim it — an unresolved alias is
    // metadata, not a model.
    Some(match resolved {
        Some(model) => ModelIdentity::ResolvedAlias {
            model: model.to_string(),
            alias: raw.to_string(),
        },
        None => ModelIdentity::UnresolvedAlias {
            alias: raw.to_string(),
        },
    })
}

/// Every alias name in the table, deduplicated across families. Used by the
/// store migration that lifts historical alias values out of the `model`
/// column.
pub fn model_alias_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for entry in MODEL_ALIASES {
        if !names.contains(&entry.alias) {
            names.push(entry.alias);
        }
    }
    names
}

/// The exact strings the alias table resolves to. Used by the pricing coverage
/// guard, which must see every alias target priced.
#[cfg(test)]
pub(crate) fn model_alias_targets() -> Vec<&'static str> {
    MODEL_ALIASES
        .iter()
        .filter_map(|entry| entry.resolves_to)
        .collect()
}
