# Strategy Pattern

Define a family of interchangeable algorithms behind a common trait. Each `impl` is a complete way to perform the *same* operation; the caller picks one based on a key derived from its input (file's language, activity's spec type) and invokes it without caring which:

```rust
trait Strategy {
    fn applies_to(&self) -> Key;
    fn run(&self, input: Input) -> Output;
}
```

A registry holds the candidates and matches `applies_to` against the input-derived key.

## When to reach for it

- **Same logical operation, multiple algorithms.** Parsing source is one operation; the algorithm differs per language. Executing an activity is one operation; the mechanics differ per `spec_type`. Callers want `extract(file)` / `execute(activity)`, not a sprawling `match`.
- **Selection is driven by data, not by call site.** The caller derives a key from the input and looks up; it does not know which impl ran.
- **Strategies are roughly fungible.** Same output shape (or close enough that one calling-side code path handles the variance).

## When NOT to

- **The "strategies" do different things.** If `impl`s return semantically different shapes, or the caller has to branch on which one ran, it's Command, not Strategy. Each `impl Tool` in `crates/orbit-tools/` is a different *operation* keyed by name, not a different *algorithm* for one operation — see `docs/design-patterns/command.md`.
- **Single impl, planned single impl.** A trait with one `impl` is just an abstraction boundary. Don't pay the `Box<dyn ...>` cost for an interface you'll never swap.
- **A few variants known up front.** `match kind { Yaml => …, Json => … }` is clearer than a trait + registry when the set is small, closed, and trivial.

## Reference: `FileExtractor`

The canonical example. One operation — *extract structural anchors from a source file* — implemented per language and per file kind. The trait at `crates/orbit-knowledge/src/extract/mod.rs:53`:

```rust
pub trait FileExtractor: Send + Sync {
    fn file_kind(&self) -> FileKind;
    fn extract(&self, source: &str) -> ExtractionResult;
}
```

The registry at `crates/orbit-knowledge/src/extract/mod.rs:59` owns `Vec<Box<dyn FileExtractor>>` and selects via `get(kind: FileKind)` by scanning for a matching `file_kind()`. Concrete strategies live in sibling modules:

- **Tree-sitter code extractors** — `rust.rs`, `python.rs`, `typescript.rs`, `go.rs`, `java.rs`, `c.rs`, `csharp.rs`, `kotlin.rs`, `javascript.rs`, `ruby.rs`
- **Document anchors** — `markdown.rs`
- **File-level capture only (no symbol leaves)** — `config.rs`, `table.rs`

The pipeline derives `FileKind` from the file, asks the registry, and calls `extract(source)`. It never branches on language.

## Reference: `ActivityExecutor`

A second textbook instance. One operation — *execute one attempt of an activity* — implemented per execution mechanic. The trait at `crates/orbit-engine/src/executor/traits.rs:17`:

```rust
pub trait ActivityExecutor: Send + Sync {
    fn spec_type(&self) -> &str;
    fn execute(&self, host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome;
}
```

The registry at `crates/orbit-engine/src/executor/registry.rs:11` keys strategies by `spec_type` in a `HashMap<String, Box<dyn ActivityExecutor>>` and dispatches on the activity's declared `spec_type`. Concrete strategies under `crates/orbit-engine/src/executor/`: `CliCommandExecutor`, `DirectAgentExecutor`, `AutomationExecutor`, plus `OrbitToolCallExecutor` (registered from `activity_job/`). The retry loop in `activity_runner` calls `registry.get(spec_type).execute(...)` without knowing which mechanic actually runs.

---

**Strategy vs Command in this codebase.** Both use `Box<dyn Trait>` and a registry. The difference is *what varies*. `FileExtractor` varies *how* to extract — every impl extracts; that's Strategy. `Tool` varies *what* to do — `which`, `adr_add`, and `pipeline_invoke` are different operations sharing only a calling convention; that's Command.
