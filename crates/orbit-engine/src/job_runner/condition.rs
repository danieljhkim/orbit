//! Expression-based step condition evaluator.
//!
//! Evaluates `StepCondition::Expr` strings against a `TemplateContext`.
//!
//! Expression syntax (post template resolution):
//!   `<lhs> == <rhs>` | `<lhs> != <rhs>`
//!   Combined with `&&` (AND, higher precedence) and `||` (OR).
//!
//! Examples:
//!   `"{{steps.plan.state.status}} == success"`
//!   `"{{steps.a.state.status}} == success && {{steps.b.output.match}} != false"`
//!   `"{{steps.a.state.status}} == success || {{steps.b.state.status}} == success"`

use orbit_types::{OrbitError, StepCondition};

use crate::template::{self, TemplateContext};

/// Evaluate a step condition against the given template context.
///
/// Keyword variants (`Always`, `OnSuccess`, etc.) are evaluated with the
/// provided `keyword_eval` closure, which lets callers keep their existing
/// sequential/DAG keyword logic. `Expr` variants resolve templates and
/// evaluate the resulting boolean expression.
pub(crate) fn evaluate_condition(
    condition: &StepCondition,
    ctx: &TemplateContext,
    keyword_eval: impl FnOnce(&StepCondition) -> bool,
) -> Result<bool, OrbitError> {
    match condition {
        StepCondition::Expr(expr) => {
            let resolved = template::render(expr, ctx)?;
            evaluate_expr(&resolved)
        }
        _ => Ok(keyword_eval(condition)),
    }
}

/// Parse and evaluate a resolved boolean expression.
///
/// Grammar (informal):
///   expr     = or_expr
///   or_expr  = and_expr ('||' and_expr)*
///   and_expr = atom ('&&' atom)*
///   atom     = value ('==' | '!=') value
///   value    = non-whitespace token (unquoted)
fn evaluate_expr(resolved: &str) -> Result<bool, OrbitError> {
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
    if let Some((lhs, rhs)) = atom.split_once("!=") {
        // Check != before == to avoid matching the = inside !=
        // But we need to be careful: "a != b" should match here, not "a !" + "= b"
        // split_once on "!=" is correct since != is a 2-char sequence.
        Ok(lhs.trim() != rhs.trim())
    } else if let Some((lhs, rhs)) = atom.split_once("==") {
        Ok(lhs.trim() == rhs.trim())
    } else {
        Err(OrbitError::InvalidInput(format!(
            "condition atom must contain '==' or '!=', got: '{atom}'"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_eq() {
        assert!(evaluate_expr("success == success").unwrap());
        assert!(!evaluate_expr("success == failed").unwrap());
    }

    #[test]
    fn test_simple_neq() {
        assert!(evaluate_expr("success != failed").unwrap());
        assert!(!evaluate_expr("success != success").unwrap());
    }

    #[test]
    fn test_and() {
        assert!(evaluate_expr("a == a && b == b").unwrap());
        assert!(!evaluate_expr("a == a && b == c").unwrap());
    }

    #[test]
    fn test_or() {
        assert!(evaluate_expr("a == b || c == c").unwrap());
        assert!(!evaluate_expr("a == b || c == d").unwrap());
    }

    #[test]
    fn test_precedence_and_binds_tighter() {
        // "false || true && true" → false || (true && true) → true
        assert!(evaluate_expr("a == b || c == c && d == d").unwrap());
        // "true && false || true" → (true && false) || true → true
        assert!(evaluate_expr("a == a && b == c || d == d").unwrap());
        // "false && true || false" → (false && true) || false → false
        assert!(!evaluate_expr("a == b && c == c || d == e").unwrap());
    }

    #[test]
    fn test_whitespace_handling() {
        assert!(evaluate_expr("  success  ==  success  ").unwrap());
        assert!(evaluate_expr("a == a  &&  b == b").unwrap());
    }

    #[test]
    fn test_invalid_atom() {
        assert!(evaluate_expr("no_operator_here").is_err());
    }

    #[test]
    fn test_evaluate_condition_keyword() {
        let ctx = TemplateContext::default();
        let result = evaluate_condition(&StepCondition::Always, &ctx, |_| true).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_expr() {
        let ctx = TemplateContext::default();
        let condition = StepCondition::Expr("success == success".to_string());
        let result = evaluate_condition(&condition, &ctx, |_| false).unwrap();
        assert!(result);
    }
}
