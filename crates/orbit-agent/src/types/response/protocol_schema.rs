//! The single canonical definition of Orbit's agent response envelope frame.
//!
//! [ORB-10746] Two consumers must agree on this frame: the Rust parser in
//! [`super::envelope`], and any provider transport able to *enforce* it at the
//! output boundary (today Claude's `--json-schema` structured output). Before
//! this module the frame existed only as prose in
//! `crate::providers::common::ORBIT_RESPONSE_CONTRACT`, so compliance was a
//! matter of model obedience — `jrun-20260812-0312-9` spent ~11.5 minutes and
//! $3.17 on completed work that ended in a prose summary and was rejected at
//! the completion guard.
//!
//! The prose contract stays as human-readable guidance. This module is what
//! actually binds the shape.

use serde_json::{Value, json};

/// The only protocol version Orbit speaks. Shared with [`super::envelope`] so
/// the schema handed to a provider and the parser that validates the reply
/// cannot disagree about what version means.
pub const RESPONSE_ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// The recognized `status` tokens. All three *terminate* the protocol; which
/// of them may checkpoint a step is a control-plane decision owned by the CLI
/// runner ([ORB-10733]), not by this frame.
pub const RESPONSE_ENVELOPE_STATUSES: [&str; 3] = ["success", "failed", "timeout"];

/// Orbit's response envelope as a JSON Schema, for providers that can
/// constrain their own output.
///
/// # What this deliberately does not express
///
/// The status/error correlation — `failed` requiring a non-empty `error.code`
/// — is *not* here, and cannot be. Expressing it needs a conditional
/// subschema, and Anthropic's structured-output schema subset rejects those:
///
/// ```text
/// API Error: 400 tools.0.custom.input_schema: input_schema does not support
/// oneOf, allOf, or anyOf at the top level
/// ```
///
/// That correlation therefore stays where it already was, in
/// [`super::envelope`]'s parse checks. This is a constraint of the provider's
/// schema subset, not a choice about where validation belongs — and it is why
/// the Rust checks remain load-bearing rather than redundant.
///
/// `error` is typed as an object and omitted on success. The subset cannot
/// express "object on failure, absent on success" without a top-level
/// conditional, so this is the tightest type the provider will accept.
/// `durationMs` is omitted entirely and rides through on unconstrained
/// additional properties.
pub fn response_envelope_json_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schemaVersion": {
                "const": RESPONSE_ENVELOPE_SCHEMA_VERSION,
                "description": "Orbit response protocol version. Always 1.",
            },
            "status": {
                "type": "string",
                "enum": RESPONSE_ENVELOPE_STATUSES,
                "description":
                    "Terminal outcome of the invocation. Use \"failed\" when the work could \
                     not be completed; do not report \"success\" for partial work.",
            },
            "result": {
                "type": "object",
                "description":
                    "Result payload. Always an object, never null — use {} for activities \
                     whose output is purely durable side effects.",
            },
            "error": {
                "type": "object",
                "description":
                    "Present only when status is \"failed\" or \"timeout\". Object with \
                     non-empty \"code\" and \"message\". Omit this field when status is \
                     \"success\".",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Stable machine-readable error code.",
                    },
                    "message": {
                        "type": "string",
                        "description": "Human-readable error message.",
                    },
                },
                "required": ["code", "message"],
            },
        },
        "required": ["schemaVersion", "status", "result"],
    })
}

/// The schema serialized for use as a CLI argument value.
pub fn response_envelope_json_schema_arg() -> String {
    // The schema is a closed literal above, so this cannot fail; serializing a
    // `Value` only errors on non-string map keys or a custom `Serialize` impl.
    response_envelope_json_schema().to_string()
}
