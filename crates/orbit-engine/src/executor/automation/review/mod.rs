mod check;
mod sync_review;

pub(super) use check::check_task_value;
pub(super) use sync_review::sync_batch_review_to_github;

pub(crate) fn normalize_review_decision(value: &str) -> String {
    match value.trim().to_ascii_uppercase().as_str() {
        "APPROVED" | "APPROVE" => "APPROVED".to_string(),
        "REQUEST-CHANGES" | "REQUEST_CHANGES" | "CHANGES_REQUESTED" => {
            "CHANGES_REQUESTED".to_string()
        }
        other => other.to_string(),
    }
}
