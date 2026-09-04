//! TOML text helpers shared by every crate that renders a config file from
//! strings it did not author.

/// Escape `value` for embedding inside a TOML basic (double-quoted) string,
/// per the TOML spec's basic-string escape set.
///
/// Anything rendered into a `"..."` literal must pass through here: a value
/// that carries a quote, a backslash, or a control character is otherwise
/// either invalid TOML or, worse, valid TOML with a different meaning.
pub fn escape_basic_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            control if (control as u32) < 0x20 || control as u32 == 0x7f => {
                escaped.push_str(&format!("\\u{:04X}", control as u32));
            }
            other => escaped.push(other),
        }
    }
    escaped
}
