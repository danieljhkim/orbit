use orbit_common::types::{FsOperation, OrbitError, PolicyDef};

use crate::evaluator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsPolicyEvaluation {
    pub profile: String,
    pub operation: FsOperation,
    pub path: String,
    pub allowed: bool,
    pub matched_rule: String,
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    def: PolicyDef,
}

impl PolicyEngine {
    pub fn from_def(def: &PolicyDef) -> Result<Self, OrbitError> {
        def.validate()?;
        Ok(Self { def: def.clone() })
    }

    pub fn check(
        &self,
        profile: impl Into<String>,
        operation: FsOperation,
        path: impl Into<String>,
    ) -> Result<FsPolicyEvaluation, OrbitError> {
        let profile = profile.into();
        let path = path.into();
        let result = evaluator::evaluate(&self.def, &profile, operation, &path)?;
        Ok(FsPolicyEvaluation {
            profile,
            operation,
            path,
            allowed: result.allowed,
            matched_rule: result.matched_rule,
        })
    }

    pub fn def(&self) -> &PolicyDef {
        &self.def
    }
}
