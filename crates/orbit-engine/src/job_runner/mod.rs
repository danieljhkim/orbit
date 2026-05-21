pub(crate) mod condition;

pub use condition::evaluate_bool_expr;

#[cfg(test)]
mod tests;
