//! PR automation state machine split across focused seams for maintainability.
//! `attribution` owns Review/Done actor labels for ship handoffs; `body` owns PR title/body rendering helpers; `open` owns branch freshness/push/PR creation/review-thread updates; `merge` owns approved-PR merge, remote cleanup, Done updates, and scoreboard reconciliation. Test-only modules mirror the same seams.

mod attribution;
mod body;
mod merge;
mod open;

#[cfg(test)]
mod tests;

pub(in crate::executor::automation) use merge::git_merge;
#[allow(unused_imports)]
pub(super) use open::open_batch_pr;
pub(in crate::executor::automation) use open::pr_open;
