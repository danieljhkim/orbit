//! In-memory working graph that diverges from the persisted `.orbit/knowledge/`
//! as edits accumulate during an activity run.

mod model;
mod ops;
mod rewrite;
mod versioning;

pub use model::{
    LeafEdit, LeafVersionChain, MoveResult, WorkingGraph, WorkingLeaf, WriteError, WriteResult,
};
