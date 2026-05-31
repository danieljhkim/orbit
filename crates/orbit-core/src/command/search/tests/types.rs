use super::*;
use serde_json::json;

#[test]
fn search_modes_serialize_with_public_flag_names() {
    assert_eq!(
        serde_json::to_value(GlobalSearchMode::Lexical).expect("serialize mode"),
        json!("lexical")
    );
    assert_eq!(
        serde_json::to_value(GlobalSearchMode::Hybrid).expect("serialize mode"),
        json!("hybrid")
    );
    assert_eq!(
        serde_json::to_value(GlobalSearchMode::Neighbor).expect("serialize mode"),
        json!("neighbor")
    );
}
