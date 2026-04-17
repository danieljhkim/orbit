use orbit_engine::{ReconcileOutcome, reconcile_once};
use orbit_types::OrbitError;

use crate::OrbitRuntime;

impl OrbitRuntime {
    pub fn reconcile_once(&self, dry_run: bool) -> Result<ReconcileOutcome, OrbitError> {
        reconcile_once(self, dry_run)
    }
}
