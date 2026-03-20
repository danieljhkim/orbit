use crate::context::{AttemptOutcome, EngineHost, ExecutionContext};

pub trait ActivityExecutor: Send + Sync {
    fn spec_type(&self) -> &str;
    fn execute(&self, host: &dyn EngineHost, execution: &ExecutionContext) -> AttemptOutcome;
}
