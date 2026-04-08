pub(crate) fn normalize_review_decision(value: &str) -> String {
    match value.trim().to_ascii_uppercase().as_str() {
        "APPROVED" | "APPROVE" => "APPROVED".to_string(),
        "REQUEST-CHANGES" | "REQUEST_CHANGES" | "CHANGES_REQUESTED" => {
            "CHANGES_REQUESTED".to_string()
        }
        other => other.to_string(),
    }
}
