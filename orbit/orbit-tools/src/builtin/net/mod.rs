pub mod http;

use crate::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(http::NetHttpTool);
}
