use std::str::FromStr;

use super::super::AgentFamily;

#[test]
fn agent_family_serializes_as_lowercase_and_rejects_aliases() {
    assert_eq!(
        serde_json::to_string(&AgentFamily::Gemini).expect("serialize family"),
        "\"gemini\""
    );
    assert!(AgentFamily::from_str("pro").is_err());
}
