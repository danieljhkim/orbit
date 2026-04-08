use crate::types::AgentOperation;

pub(crate) struct MockAgentCliTransport;

impl MockAgentCliTransport {
    pub(crate) fn args(&self, operation: &AgentOperation) -> Vec<String> {
        match operation {
            AgentOperation::Activity { activity_id } => vec![
                "run".to_string(),
                "--target-type".to_string(),
                "activity".to_string(),
                "--target-id".to_string(),
                activity_id.clone(),
                "--mode".to_string(),
                "activity".to_string(),
                "--output".to_string(),
                "json".to_string(),
            ],
            AgentOperation::Job {
                job_id,
                activity_id,
            } => vec![
                "run".to_string(),
                "--target-type".to_string(),
                "activity".to_string(),
                "--target-id".to_string(),
                activity_id.clone(),
                "--job-id".to_string(),
                job_id.clone(),
                "--mode".to_string(),
                "job".to_string(),
                "--output".to_string(),
                "json".to_string(),
            ],
        }
    }

    pub(crate) fn stdin(&self, envelope_json: &[u8]) -> Vec<u8> {
        envelope_json.to_vec()
    }
}
