mod client;
mod patch_match;
mod thread_sync;

pub(in crate::executor::automation) use thread_sync::sync_batch_review_to_github;
