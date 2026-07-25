// ORB-10354: crew aliases must never be mistaken for exact model strings.

use chrono::{DateTime, Utc};

use crate::types::TokenUsage;
use crate::types::model_identity::{
    CLAUDE_FABLE_ALIAS_TARGET, CLAUDE_OPUS_ALIAS_TARGET, CLAUDE_SONNET_ALIAS_TARGET, ModelIdentity,
    classify_model_string, model_alias_names, model_alias_targets,
};
use crate::types::pricing::derive_cost_usd;

#[test]
fn resolves_claude_crew_aliases_to_exact_strings() {
    for (alias, expected) in [
        ("opus", CLAUDE_OPUS_ALIAS_TARGET),
        ("sonnet", CLAUDE_SONNET_ALIAS_TARGET),
        ("fable", CLAUDE_FABLE_ALIAS_TARGET),
    ] {
        assert_eq!(
            classify_model_string(Some("claude"), alias),
            Some(ModelIdentity::ResolvedAlias {
                model: expected.to_string(),
                alias: alias.to_string(),
            }),
            "alias {alias} must resolve to {expected}",
        );
    }
}

#[test]
fn keeps_an_exact_model_string_verbatim() {
    assert_eq!(
        classify_model_string(Some("claude"), "claude-opus-4-8[1m]"),
        Some(ModelIdentity::Exact("claude-opus-4-8[1m]".to_string()))
    );
    assert_eq!(
        classify_model_string(Some("codex"), " gpt-5.6-terra "),
        Some(ModelIdentity::Exact("gpt-5.6-terra".to_string())),
        "surrounding whitespace is trimmed, the string itself is untouched"
    );
}

#[test]
fn an_unresolvable_alias_carries_no_model_string() {
    let identity = classify_model_string(Some("gemini"), "pro").expect("classified");
    assert_eq!(
        identity,
        ModelIdentity::UnresolvedAlias {
            alias: "pro".to_string()
        }
    );
    assert_eq!(
        identity.model(),
        None,
        "never guessed into the model column"
    );
    assert_eq!(identity.alias(), Some("pro"));
}

#[test]
fn an_alias_stays_an_alias_under_a_foreign_family() {
    // A `claude` alias arriving with a codex family label is malformed input;
    // recording it as an exact codex model would be worse than recording it as
    // an unresolved alias.
    assert_eq!(
        classify_model_string(Some("codex"), "opus"),
        Some(ModelIdentity::UnresolvedAlias {
            alias: "opus".to_string()
        })
    );
}

#[test]
fn resolves_an_unambiguous_alias_without_a_family() {
    assert_eq!(
        classify_model_string(None, "OPUS"),
        Some(ModelIdentity::ResolvedAlias {
            model: CLAUDE_OPUS_ALIAS_TARGET.to_string(),
            alias: "OPUS".to_string(),
        }),
        "alias matching is case-insensitive; the raw spelling is kept as provenance"
    );
}

#[test]
fn blank_model_strings_classify_as_absent() {
    assert_eq!(classify_model_string(Some("claude"), ""), None);
    assert_eq!(classify_model_string(Some("claude"), "   "), None);
}

#[test]
fn alias_names_cover_the_crew_config_aliases() {
    let names = model_alias_names();
    for alias in ["opus", "sonnet", "fable", "pro"] {
        assert!(
            names.contains(&alias),
            "crew alias {alias} must be known to the store migration; got {names:?}",
        );
    }
}

/// The point of resolving an alias at ingest is that the resulting string
/// prices. A resolution target with no price row would trade one silent
/// zero-cost hole for another.
#[test]
fn every_alias_target_is_priced() {
    let usage = TokenUsage {
        input: 1_000,
        output: 1_000,
        ..TokenUsage::default()
    };
    let at: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-07-24T00:00:00Z")
        .expect("valid rfc3339 fixture timestamp")
        .with_timezone(&Utc);
    for target in model_alias_targets() {
        assert!(
            derive_cost_usd(target, at, &usage).is_some(),
            "alias target {target} has no covering price row in model_prices.yaml",
        );
    }
}

/// The aliases themselves stay unpriced on purpose (ADR-0245): pricing them
/// would paper over an alias that reached the `model` column.
#[test]
fn alias_names_themselves_are_never_priced() {
    let usage = TokenUsage {
        input: 1_000,
        output: 1_000,
        ..TokenUsage::default()
    };
    for alias in model_alias_names() {
        assert_eq!(
            derive_cost_usd(alias, Utc::now(), &usage),
            None,
            "alias {alias} must not be priced directly",
        );
    }
}
