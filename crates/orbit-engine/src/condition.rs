//! Expression-based step condition evaluator.
//!
//! Evaluates `StepCondition::Expr` strings against a `TemplateContext`.
//!
//! Expression syntax (post template resolution):
//!   `true` | `false` | `<lhs> == <rhs>` | `<lhs> != <rhs>`
//!   Combined with `&&` (AND, higher precedence) and `||` (OR).
//!
//! Examples:
//!   `"{{steps.plan.state.status}} == success"`
//!   `"{{steps.a.state.status}} == success && {{steps.b.output.match}} != false"`
//!   `"{{steps.a.state.status}} == success || {{steps.b.state.status}} == success"`

use orbit_common::types::OrbitError;

use crate::template::{self, TemplateContext};

/// Render a boolean expression through the template engine and evaluate the
/// result. Shared between v1's `StepCondition::Expr` and v2's `when:` / loop
/// `break_when:` constructs (§4.2). The expression grammar is documented on
/// `evaluate_expr`.
pub fn evaluate_bool_expr(expr: &str, ctx: &TemplateContext) -> Result<bool, OrbitError> {
    let resolved = template::render(expr, ctx)?;
    evaluate_expr(&resolved)
}

/// Parse and evaluate a resolved boolean expression.
///
/// Grammar (informal):
///   expr     = or_expr
///   or_expr  = and_expr ('||' and_expr)*
///   and_expr = atom ('&&' atom)*
///   atom     = boolean-literal | value ('==' | '!=') value
///   boolean-literal = 'true' | 'false'
///   value    = non-whitespace token (unquoted)
// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn evaluate_expr(resolved: &str) -> Result<bool, OrbitError> {
    let or_groups: Vec<&str> = split_keep_delim(resolved, "||");
    let mut result = false;
    for group in or_groups {
        let and_atoms: Vec<&str> = split_keep_delim(group, "&&");
        let mut group_result = true;
        for atom in and_atoms {
            group_result = group_result && evaluate_atom(atom.trim())?;
        }
        result = result || group_result;
    }
    Ok(result)
}

/// Split a string by a delimiter, but only at the top level (not inside tokens).
/// Returns the segments between delimiters.
fn split_keep_delim<'a>(input: &'a str, delim: &str) -> Vec<&'a str> {
    let mut segments = Vec::new();
    let mut remaining = input;
    while let Some(pos) = remaining.find(delim) {
        segments.push(&remaining[..pos]);
        remaining = &remaining[pos + delim.len()..];
    }
    segments.push(remaining);
    segments
}

/// Evaluate a single comparison atom: `<lhs> == <rhs>` or `<lhs> != <rhs>`.
fn evaluate_atom(atom: &str) -> Result<bool, OrbitError> {
    if atom == "true" {
        Ok(true)
    } else if atom == "false" {
        Ok(false)
    } else if let Some((lhs, rhs)) = atom.split_once("!=") {
        // Check != before == to avoid matching the = inside !=
        // But we need to be careful: "a != b" should match here, not "a !" + "= b"
        // split_once on "!=" is correct since != is a 2-char sequence.
        Ok(lhs.trim() != rhs.trim())
    } else if let Some((lhs, rhs)) = atom.split_once("==") {
        Ok(lhs.trim() == rhs.trim())
    } else {
        Err(OrbitError::InvalidInput(format!(
            "condition atom must be 'true', 'false', or contain '==' or '!=', got: '{atom}'"
        )))
    }
}
