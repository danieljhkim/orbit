use super::{OrbitError, TaskDocumentUpdateParams};

pub(crate) fn unsupported_v2_operation(operation: &str) -> OrbitError {
    OrbitError::Store(format!(
        "task artifact v2 operation '{operation}' is not supported yet"
    ))
}

pub(super) fn reject_unsupported_document_fields(
    fields: &TaskDocumentUpdateParams,
) -> Result<(), OrbitError> {
    if fields.pr_status.as_ref().is_some_and(Option::is_some) {
        return Err(unsupported_v2_operation("update_task_document.pr_status"));
    }
    Ok(())
}
