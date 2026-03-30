use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentOperation {
    Activity { activity_id: String },
    Job { job_id: String, activity_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRequest {
    pub operation: AgentOperation,
    pub envelope_json: Vec<u8>,
    /// The activity's `output_schema_json`, used to pass `--json-schema` to
    /// providers that support structured output (e.g. Claude CLI).
    /// `None` or an empty object `{}` means freeform output.
    pub output_schema_json: Option<Value>,
    /// When `true`, the agent CLI should produce verbose/debug output on stderr.
    pub verbose: bool,
}

impl AgentRequest {
    pub fn activity(
        activity_id: impl Into<String>,
        envelope_json: Vec<u8>,
        output_schema_json: Option<Value>,
    ) -> Self {
        Self {
            operation: AgentOperation::Activity {
                activity_id: activity_id.into(),
            },
            envelope_json,
            output_schema_json,
            verbose: false,
        }
    }

    pub fn job(
        job_id: impl Into<String>,
        activity_id: impl Into<String>,
        envelope_json: Vec<u8>,
        output_schema_json: Option<Value>,
    ) -> Self {
        Self {
            operation: AgentOperation::Job {
                job_id: job_id.into(),
                activity_id: activity_id.into(),
            },
            envelope_json,
            output_schema_json,
            verbose: false,
        }
    }

    /// Set the verbose flag on this request.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }
}
