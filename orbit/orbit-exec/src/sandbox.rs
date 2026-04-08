use orbit_types::OrbitError;

use crate::runner::ExecRequest;

pub trait Sandbox {
    fn validate(&self, req: &ExecRequest) -> Result<(), OrbitError>;
}

#[derive(Debug, Default)]
pub struct NoSandbox;

impl Sandbox for NoSandbox {
    fn validate(&self, _req: &ExecRequest) -> Result<(), OrbitError> {
        Ok(())
    }
}
