pub mod now;
pub mod sleep;

use crate::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(now::TimeNowTool);
    registry.register(sleep::TimeSleepTool);
}
