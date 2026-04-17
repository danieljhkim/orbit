//! `check_duel_review_decision` automation.
//!
//! Gates the duel fix loop on the arbiter verdict, NOT the raw reviewer
//! output. This is the key behavioral difference from
//! `check_batch_review_decision` — the duel workflow treats the arbiter as
//! authoritative, so a "nitpicky" or fabricated reviewer comment cannot
//! trigger another fix iteration on its own.
//!
//! The arbiter's output is piped into this step's input via the normal
//! pipeline patch flow. We read the top-level `decision` field and emit
//! `loop_exit: true` iff it is `APPROVED`.

use orbit_types::{Decision, OrbitError};
use serde_json::{Value, json};

use super::input::required_input_string;

/// Extract the arbiter decision from the current input. The arbiter
/// writes a top-level `decision` string (`APPROVED` or `REQUEST_CHANGES`)
/// and we reject any other value loudly — silently treating an unknown
/// decision as one or the other would corrupt the fix-loop gating logic.
fn parse_decision(input: &Value) -> Result<Decision, OrbitError> {
    let raw = required_input_string(input, "decision")?;
    match raw {
        "APPROVED" => Ok(Decision::Approved),
        "REQUEST_CHANGES" => Ok(Decision::RequestChanges),
        other => Err(OrbitError::Execution(format!(
            "check_duel_review_decision: unexpected arbiter decision '{other}' \
             (expected APPROVED or REQUEST_CHANGES)"
        ))),
    }
}

pub(super) fn check_duel_review_decision(input: &Value) -> Result<Value, OrbitError> {
    let decision = parse_decision(input)?;
    let loop_exit = matches!(decision, Decision::Approved);
    Ok(json!({
        "review_decision": match decision {
            Decision::Approved => "APPROVED",
            Decision::RequestChanges => "REQUEST_CHANGES",
        },
        "loop_exit": loop_exit,
    }))
}
