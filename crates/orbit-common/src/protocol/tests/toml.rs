use super::super::toml::escape_basic_string;

#[test]
fn escapes_every_basic_string_hazard() {
    assert_eq!(escape_basic_string("plain"), "plain");
    assert_eq!(escape_basic_string("say \"hi\""), "say \\\"hi\\\"");
    assert_eq!(escape_basic_string("back\\slash"), "back\\\\slash");
    assert_eq!(escape_basic_string("a\nb\tc"), "a\\nb\\tc");
    assert_eq!(escape_basic_string("\u{01}"), "\\u0001");
}

/// The escaped form must parse back to the original through a real TOML
/// parser, which is the only contract that matters.
#[test]
fn round_trips_through_a_toml_parser() {
    for value in ["laptop\"", "\"\"\"open", "tab\there", "x\\y", "\u{7f}"] {
        let document = format!("label = \"{}\"\n", escape_basic_string(value));
        let parsed: toml::Value = toml::from_str(&document).expect("valid TOML");
        assert_eq!(parsed["label"].as_str(), Some(value), "{document}");
    }
}
