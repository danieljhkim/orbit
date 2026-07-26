---
type: pattern
summary: "Command Pattern"
last_validated: 2026-07-26
---
# Command Pattern

In this codebase, Command = the `Tool` trait at `crates/orbit-tools/src/lib.rs:243`:

```rust
pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolSchema;
    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError>;
}
```

The registry at `crates/orbit-tools/src/registry.rs:31` stores `Arc<dyn Tool>` keyed by `ToolSchema::name`. Adding a tool means: writing a struct, `impl Tool`, registering in `builtin::register_builtins`. The dispatcher never changes.

Two codebase-specific shapes carry non-obvious lessons; everything else is straightforward `impl Tool`.

## Reference: host-action dispatcher (`OrbitPipelineInvokeTool`)

The most common shape in the `orbit.*` namespace. The struct declares the schema; execution delegates to an `OrbitToolHost` via an action enum. From `crates/orbit-tools/src/builtin/orbit/pipeline/invoke.rs:7`:

```rust
pub struct OrbitPipelineInvokeTool;

impl Tool for OrbitPipelineInvokeTool {
    fn schema(&self) -> ToolSchema { /* params + identity_params() */ }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::PipelineInvoke)
    }
}
```

`execute_host_action` (`orbit/mod.rs:275`) resolves the caller's identity, requires a host on the context, and forwards `(action, input, agent, model, reservation_owner)` into the runtime.

Patterns to copy:

- **A new tool of this kind lands in three places.** New struct in `orbit/<area>/<verb>.rs`, new variant in `OrbitBuiltinAction`, new match arm in the host's `execute()`. The dispatcher and registry are untouched.
- **Schema in `orbit-tools`; logic in `orbit-core`.** This is the rule that keeps `orbit-tools` free of runtime / store dependencies per the architecture diagram in `CLAUDE.md`. If your tool needs the task store, the activity-job engine, or sandboxed exec, it must dispatch through the host — don't pull those deps into `orbit-tools`.
- **Remote MCP-only adapters stay in `orbit-remote`.** Global host/workspace discovery and local-derived graph definitions are not generic `ToolRegistry` commands. Their schema, policy, and execution live together in Remote and are composed over the generic MCP kernel; adding one does not add an `OrbitBuiltinAction` or Core `run_tool` arm.

## Former compatibility shim: `graph.history`

The former `OrbitGraphHistoryTool` compatibility stub is no longer present. The registry records that ORB-00391 removed the v1 `orbit-knowledge` graph builtins and `graph.history` stub, while ORB-10325 keeps v2 graph access exclusively on `orbit graph` (`crates/orbit-tools/src/builtin/orbit/mod.rs:120`). Do not use the retired shim shape as a template for new tools.

---

**Not Command — same code shape, different role.** The codebase also uses `Box<dyn Trait>` + registry where every `impl` is a parallel algorithm for *the same* operation (parse Rust vs parse Python). That's Strategy, not Command — selection is by input-derived key, not by caller naming the operation. The current load-bearing example is `ExtractorRegistry` (`crates/orbit-graph/src/sync/scanner.rs:183`, `Vec<Box<dyn Extractor>>` + `supports()` predicate). The engine's `ActivityExecutor` + `ActivityExecutorRegistry` (`HashMap<String, Box<dyn _>>` keyed by `spec_type`) used to be the second example; [ORB-10395] deleted it — v2 dispatch selects a handler from a typed spec enum instead, so don't reach for a string-keyed executor registry in the engine.
