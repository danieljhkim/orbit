use std::collections::HashMap;

use orbit_types::{ExecutorDef, ExecutorType};
use tracing::warn;

use super::agent::AgentExecutor;
use super::automation::AutomationExecutor;
use super::cli_command::CliCommandExecutor;
use super::direct_agent::DirectAgentExecutor;
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

    /// Register a named executor, keyed by the given name rather than spec_type.
    pub fn register_named(
        &mut self,
        name: String,
        executor: Box<dyn ActivityExecutor>,
    ) -> Option<Box<dyn ActivityExecutor>> {
        self.executors.insert(name, executor)
    }

    pub fn register_builtins(&mut self) {
        let _ = self.register(AgentExecutor::new());
        let _ = self.register(CliCommandExecutor);
        let _ = self.register(AutomationExecutor);
        let _ = self.register(crate::v2::OrbitToolCallExecutor);
    }

    /// Load executor definitions from YAML resources. Entries override builtins by name.
    pub fn load_from_defs(&mut self, defs: &[ExecutorDef]) {
        for def in defs {
            match def.executor_type {
                ExecutorType::AgentCli => {
                    if def.command.is_some() {
                        let executor = AgentExecutor::from_executor_def(def.clone());
                        self.register_named(def.name.clone(), Box::new(executor));
                    } else {
                        warn!(
                            executor_name = %def.name,
                            "agent_cli executor def missing 'command' field, skipping"
                        );
                    }
                }
                ExecutorType::DirectAgent => {
                    if def.command.is_some() {
                        let executor = DirectAgentExecutor::from_executor_def(def.clone());
                        self.register_named(def.name.clone(), Box::new(executor));
                    } else {
                        warn!(
                            executor_name = %def.name,
                            "direct_agent executor def missing 'command' field, skipping"
                        );
                    }
                }
                ExecutorType::CliCommand => {
                    self.register_named(def.name.clone(), Box::new(CliCommandExecutor));
                }
            }
        }
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
