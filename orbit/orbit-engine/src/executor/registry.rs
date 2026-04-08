use std::collections::HashMap;
use std::sync::OnceLock;

use super::agent::AgentExecutor;
use super::automation::AutomationExecutor;
use super::cli_command::CliCommandExecutor;
use super::traits::ActivityExecutor;

pub struct ActivityExecutorRegistry {
    executors: HashMap<String, Box<dyn ActivityExecutor>>,
}

impl ActivityExecutorRegistry {
    pub fn new() -> Self {
        Self {
            executors: HashMap::new(),
        }
    }

    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register_builtins();
        registry
    }

    pub fn register<E>(&mut self, executor: E) -> Option<Box<dyn ActivityExecutor>>
    where
        E: ActivityExecutor + 'static,
    {
        self.executors
            .insert(executor.spec_type().to_string(), Box::new(executor))
    }

    pub fn register_builtins(&mut self) {
        let _ = self.register(AgentExecutor);
        let _ = self.register(CliCommandExecutor);
        let _ = self.register(AutomationExecutor);
    }

    pub fn get(&self, spec_type: &str) -> Option<&dyn ActivityExecutor> {
        self.executors.get(spec_type).map(Box::as_ref)
    }

    pub fn supported_spec_types(&self) -> Vec<&str> {
        let mut spec_types = self
            .executors
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        spec_types.sort_unstable();
        spec_types
    }
}

impl Default for ActivityExecutorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn builtin_activity_executor_registry() -> &'static ActivityExecutorRegistry {
    static BUILTIN_EXECUTORS: OnceLock<ActivityExecutorRegistry> = OnceLock::new();
    BUILTIN_EXECUTORS.get_or_init(ActivityExecutorRegistry::with_builtins)
}
