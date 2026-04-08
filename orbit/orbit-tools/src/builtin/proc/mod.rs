pub mod spawn;
pub mod which;

use crate::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(spawn::ProcSpawnTool);
    registry.register(which::ProcWhichTool);
}
