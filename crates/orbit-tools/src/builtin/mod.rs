// This implementation tree is crate-private; some planned or
// connector-specific tools are intentionally not registered by default yet.
#![allow(dead_code)]

pub mod git;
pub mod github;
pub mod net;
pub mod orbit;
pub mod proc;
pub mod time;

use crate::ToolRegistry;

pub fn register_builtins(registry: &mut ToolRegistry) {
    git::register(registry);
    github::register(registry);
    orbit::register(registry);
    proc::register(registry);
    time::register(registry);
    net::register(registry);
}
