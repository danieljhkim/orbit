#![allow(missing_docs)]

// The concrete test modules for planning_duel/* live under the planning_duel crate module's
// own tests/ envelope (planning_duel/tests/mod.rs + artifacts.rs etc), pulled in via
// #[cfg(test)] mod tests; in planning_duel/mod.rs.
// This thin module satisfies `mod planning_duel;` in duel/tests/mod.rs to follow the
// uniform test layout rule for all direct children of duel/.
