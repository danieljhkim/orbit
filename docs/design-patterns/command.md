---
type: pattern
summary: "Command Pattern"
---
# Command Pattern

In this codebase, Command = the `Tool` trait at `crates/orbit-tools/src/lib.rs:225`:

```rust
pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolSchema;
    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError>;
}
```

The registry at `crates/orbit-tools/src/registry.rs:10` stores `Arc<dyn Tool>` keyed by `ToolSchema::name`. Adding a tool means: writing a struct, `impl Tool`, registering in `builtin::register_builtins`. The dispatcher never changes.

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

`execute_host_action` (`orbit/mod.rs:204`) resolves the caller's identity, requires a host on the context, and forwards `(action, input, agent, model, reservation_owner)` into the runtime.

Patterns to copy:

- **A new tool of this kind lands in three places.** New struct in `orbit/<area>/<verb>.rs`, new variant in `OrbitBuiltinAction`, new match arm in the host's `execute()`. The dispatcher and registry are untouched.
- **Schema in `orbit-tools`; logic in `orbit-core`.** This is the rule that keeps `orbit-tools` free of runtime / store dependencies per the architecture diagram in `CLAUDE.md`. If your tool needs the task store, the activity-job engine, or sandboxed exec, it must dispatch through the host — don't pull those deps into `orbit-tools`.

## Reference: compatibility shim (`OrbitGraphHistoryTool`)

A tool whose entire `execute()` is a structured deprecation error. Used when a tool is removed but the name should still resolve to an actionable redirect rather than "tool not found." From `crates/orbit-tools/src/builtin/orbit/graph_history.rs:10`:

```rust
impl Tool for OrbitGraphHistoryTool {
    fn schema(&self) -> ToolSchema { /* keeps the old name + params */ }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = super::required_string(&input, &["selector"], "selector")?;
        let _: Selector = selector_str.parse()
            .map_err(|error| OrbitError::InvalidInput(format!("{error}")))?;
        Err(OrbitError::InvalidInput(REMOVED_GRAPH_HISTORY_MESSAGE.to_string()))
    }
}
```

Patterns to copy:

- **Keep the schema; replace the body.** Agents discover tools by name. A tool that vanishes leaves a worse trail than one that returns a redirect.
- **Validate inputs before returning the deprecation error.** Malformed callers get "invalid selector"; correct callers get the redirect message. Easier to diagnose.
- **Redirect text in a `pub const` next to its replacement.** `REMOVED_GRAPH_HISTORY_MESSAGE` lives in `orbit-knowledge::workflows::observe` — alongside whatever still implements the underlying capability — and the shim imports it. Don't inline the message in the tool body; the replacement guidance rots if it's not near the replacement.

---

**Not Command — same code shape, different role.** The codebase also uses `Box<dyn Trait>` + registry where every `impl` is a parallel algorithm for *the same* operation (parse Rust vs parse Python; run via CLI vs via direct-agent). That's Strategy, not Command — selection is by input-derived key, not by caller naming the operation. If that's what you're building, the load-bearing examples are `FileExtractor` (`crates/orbit-knowledge/src/extract/mod.rs:53`, `Vec<Box<dyn _>>` + `applies_to()` predicate) and `ActivityExecutor` (`crates/orbit-engine/src/executor/traits.rs:17`, `HashMap<String, Box<dyn _>>` keyed by `spec_type`).
