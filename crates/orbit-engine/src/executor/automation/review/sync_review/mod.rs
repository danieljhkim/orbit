mod client;
mod patch_match;
mod thread_sync;

#[cfg(test)]
mod tests;

pub(in crate::executor::automation) use thread_sync::sync_batch_review_to_github;
