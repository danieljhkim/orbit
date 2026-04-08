pub mod fs;
pub mod git;
pub mod github;
pub mod net;
pub mod orbit;
pub mod proc;
pub mod time;

use crate::ToolRegistry;

pub fn register_builtins(registry: &mut ToolRegistry) {
    fs::register(registry);
    git::register(registry);
    github::register(registry);
    orbit::register(registry);
    proc::register(registry);
    time::register(registry);
    net::register(registry);
}
