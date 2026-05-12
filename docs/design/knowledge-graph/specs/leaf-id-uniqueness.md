# Spec: Leaf ID Uniqueness

**Last updated:** 2026-05-12

Every extractor that emits `ExtractedLeaf` records must finish with leaf IDs that are unique within the extracted file. The selector envelope remains `symbol:{path}#{qualified_name}:{kind}`; the contract is that no two leaves for the same `path` share the same `(qualified_name, kind)` after extractor finalization.

## Why This Exists

The SQLite sidecar stores graph nodes keyed by `node.id`, while the JSON fallback keeps the full `graph.leaves` vector. When two leaves in one file derive the same ID, SQL collapses them and fallback preserves them, so read paths cannot be equivalent. This surfaced as duplicate-leaf carve-outs in search/show work and as `graph.overview` undercounts on real Python and Rust corpora. Leaf IDs therefore have to be unique before persistence, not repaired by SQL read paths.

## 1. Invariant

For every extracted file:

- `LeafNode::node_id` derived from `symbol:{path}#{qualified_name}:{kind}` is unique for every leaf in that file.
- `parent_qualified_name` and `children_qualified_names` point at finalized qualified names, not pre-finalization names.
- The finalizer runs after language extraction and before pipeline parent-child wiring.
- A duplicate after finalization is an extractor bug. SQL writers and readers must not rely on `INSERT OR REPLACE`, aggregation, or fallback-specific behavior to hide it.

The invariant is per file because `path` is already part of the leaf ID. Equivalently: within a file, `(qualified_name, kind)` is unique.

## 2. Scheme

The chosen scheme is **language-natural qualifier + deterministic occurrence suffix**.

### Layer A: language-natural qualifiers

Each extractor first emits the most precise human-readable qualified name that the language syntax makes cheap and stable.

| Language | Required qualifier rule | Natural duplicate covered |
|----------|-------------------------|---------------------------|
| Python | Classes and methods include the dotted enclosing class chain. A method `save` inside class `User` is `User.save`; a method inside nested class `Outer.Inner` is `Outer.Inner.method`. Top-level functions stay bare. | Same method name in different classes; nested class methods. |
| Rust | Inherent impl blocks use `<Type>`; trait impl blocks use `<Type as Trait>`. Methods under those blocks use `<Type>::method` or `<Type as Trait>::method`. | `impl Foo` and `impl Trait for Foo` in the same file; methods with the same name on those impls. |
| Java | Methods append parameter arity to the parent-qualified method name: `Parent::method#arity`. | Overloaded methods with different arity. |
| TypeScript / TSX | Class methods append parameter arity: `Parent::method#arity`. Top-level function overload declarations keep the function name and rely on Layer B occurrence suffixes. | Class method overloads with different arity; function overload declarations. |

Other extractors keep their existing language-natural qualified names, then still pass through Layer B. If a pre-flight uniqueness fixture finds a duplicate in Go, Kotlin, Ruby, JavaScript, C, or C#, that extractor should add a stable Layer A qualifier when the language syntax exposes one cheaply; otherwise the deterministic occurrence suffix is the required backstop.

### Layer B: deterministic occurrence suffix

Every extractor calls `finalize_unique_qualified_names(leaves: &mut [ExtractedLeaf])` as its last extraction step.

The finalizer:

1. Groups leaves by `(qualified_name, kind)` within the file.
2. Leaves singletons unchanged.
3. Sorts each duplicate group deterministically by `(start_line, end_line, source_hash, original_index)`.
4. Keeps the first occurrence unchanged and rewrites the second and later occurrences to `{original}#{ordinal}`, with ordinals starting at `2`.
5. Rewrites matching `parent_qualified_name` and `children_qualified_names` references so parent-child wiring still points at the finalized names.

The suffix is an occurrence ordinal, not a source line number. It is intentionally the fallback of last resort: Layer A should handle the common human-readable cases, while Layer B makes the uniqueness invariant total for same-arity overloads, generated duplicate declarations, and future extractor regressions.

## 3. Language Coverage

| Duplicate shape | Example finalized names | Resolved by |
|-----------------|-------------------------|-------------|
| Python class methods with the same name | `User.save`, `Order.save` | Python dotted enclosing class chain. |
| Python nested class method | `Outer.Inner.run` | Python dotted enclosing class chain. |
| Rust inherent impl plus trait impl for the same type | `<Foo>`, `<Foo as Runnable>` | Rust impl qualifier. |
| Rust methods with the same name across impl blocks | `<Foo>::run`, `<Foo as Runnable>::run` | Rust impl qualifier inherited by methods. |
| Java overloads with different arity | `Client::connect#1`, `Client::connect#2` | Java arity suffix. |
| Java overloads with the same arity | `Client::connect#1`, `Client::connect#1#2` | Java arity suffix, then Layer B. |
| TypeScript function overloads | `load`, `load#2`, `load#3` | Layer B occurrence suffix. |
| TypeScript method overloads with different arity | `Service::load#1`, `Service::load#2` | TypeScript method arity suffix. |
| TypeScript method overload signatures with the same arity | `Service::load#1`, `Service::load#1#2` | TypeScript method arity suffix, then Layer B. |

## 4. Selector String Breakage

The selector envelope is unchanged, but the `qualified_name` portion is intentionally allowed to change. Callers must treat the text between the first selector `#` and the final `:{kind}` as opaque. In particular, the qualified portion may now contain `.`, `::`, `<...>`, spaces from Rust trait-impl formatting, and additional `#` characters from arity or occurrence suffixes.

| Previous affected selector shape | New selector shape |
|----------------------------------|--------------------|
| `symbol:models.py#save:method` | `symbol:models.py#User.save:method` or `symbol:models.py#Order.save:method` |
| `symbol:models.py#run:method` for a nested class method | `symbol:models.py#Outer.Inner.run:method` |
| `symbol:lib.rs#Foo:impl` | `symbol:lib.rs#<Foo>:impl` |
| `symbol:lib.rs#Foo:impl` for a trait impl | `symbol:lib.rs#<Foo as Runnable>:impl` |
| `symbol:lib.rs#Foo::run:method` | `symbol:lib.rs#<Foo>::run:method` |
| `symbol:lib.rs#Foo::run:method` for a trait impl method | `symbol:lib.rs#<Foo as Runnable>::run:method` |
| `symbol:Client.java#Client::connect:method` | `symbol:Client.java#Client::connect#1:method` or `symbol:Client.java#Client::connect#2:method` |
| Repeated `symbol:service.ts#load:function` overload declarations | `symbol:service.ts#load:function`, `symbol:service.ts#load#2:function`, then higher occurrence suffixes |

Stored selectors for affected leaves are a breaking change after rebuild. Agents and scripts should rediscover affected selectors through `orbit.graph.search`, `orbit.graph.show`, or `orbit.graph.pack` instead of carrying old method/impl/overload selector strings forward.

### 4.1 In-repo selector-string sweep targets

A 2026-05-10 pre-sweep using the task plan's selector grep found hard-coded selector strings outside `docs/`, test-only paths, and fixture-only paths. The implementation pass must triage these categories:

| Area | Files | Required handling |
|------|-------|-------------------|
| Active selector examples | `crates/orbit-cli/src/command/observe/graph.rs`, `crates/orbit-core/assets/skills/orbit-create-task/SKILL.md`, `crates/orbit-core/assets/skills/orbit-graph/SKILL.md`, `website/src/content/docs/concepts/knowledge-graph.md` | Keep generic free-function examples if they remain valid; update any method/impl/overload examples that imply the old qualified-name grammar. |
| Selector parsing, lint, and batching code | `crates/orbit-common/src/utility/selector.rs`, `crates/orbit-core/src/command/task/lint.rs`, `crates/orbit-engine/src/executor/automation/batch/dispatch.rs`, `crates/orbit-engine/src/executor/automation/batch/parallel.rs`, `crates/orbit-engine/src/executor/automation/duel/planning_duel/context_files.rs`, `crates/orbit-engine/src/executor/automation/vcs/commit/mod.rs` | Preserve the envelope contract and add coverage for qualified names that contain extra `#` characters; do not parse language syntax out of the qualified portion. |
| Knowledge command/index expectations | `crates/orbit-knowledge/src/commands/refs.rs`, `crates/orbit-knowledge/src/commands/search.rs`, `crates/orbit-knowledge/src/graph/object_store.rs`, `crates/orbit-knowledge/src/graph/sqlite_index.rs`, `crates/orbit-knowledge/src/service/implementors.rs` | Update expected selectors, counts, and comments after the extractor change; simple-name fallback must strip language qualifiers and numeric selector suffixes; the service comment that names Rust impl selector shapes must mention `<Type>` / `<Type as Trait>`. |
| Benchmarks and historical transcripts | `benchmarks/graph/**`, `benchmarks/identity-key/**`, `benchmarks/knowledge-command-equivalence/run.sh` | Do not rewrite immutable historical transcripts just for selector churn. Rebaseline active benchmark scripts or expected outputs only when they assert current selector strings. |

Out-of-repo state such as per-user task context files, run traces, and copied selectors is not migratable. The compatibility path is rediscovery through current graph query responses.

## 5. Migration

The implementation that lands this spec bumps `GRAPH_SQLITE_INDEX_SCHEMA_VERSION` so readers that only understand the old SQLite sidecar return `Ok(None)` and use the JSON/object fallback instead. Existing graph directories keep working through fallback until the next full graph build. The rebuild emits new unique leaf IDs and a current SQLite sidecar.

There is no selector rewrite migration. Old selectors keep resolving only as long as the old graph snapshot is the one being read. After rebuild, affected selectors use the new qualified-name forms above.

This closes the duplicate-leaf carve-out documented in [T20260510-1], keeps the [T20260510-2] child-table shape valid because parent-child names are patched after finalization, and supersedes the [T20260510-6] overview undercount symptom with a structural fix.

## 6. Validation

The implementation must ship all of these checks:

- Per-language extractor fixtures assert that `leaf_ids.iter().collect::<HashSet<_>>().len() == leaf_ids.len()` for Python, Rust, Java, and TypeScript natural duplicate shapes.
- Sister-language pre-flight fixtures assert the same invariant for Go, Kotlin, Ruby, JavaScript, C, and C#.
- SQL/fallback equivalence tests compare leaves as multisets across the duplicate fixtures, mirroring the [T20260510-2] equivalence-test shape.
- A schema-version test verifies that old SQLite sidecars fall back gracefully and new sidecars produce the same leaves as the navigator.
- Live corpora verification confirms `orbit.graph.overview` totals match build-log leaf counts in both SQL and forced-fallback modes for python-medium and rust-medium.

## Agent Signature

Drafted by Codex (`gpt-5.5`) on 2026-05-10 for [T20260510-7].

## Task References

- **[T20260509-71]** — SQLite reader version contract: unsupported sidecar versions return `Ok(None)` and fall back.
- **[T20260510-1]** — Search equivalence work that documented the duplicate-leaf carve-out this spec removes.
- **[T20260510-2]** — Show/children equivalence work whose child-table shape remains valid after finalized-name patching.
- **[T20260510-6]** — Overview undercount symptom superseded by this structural leaf-ID fix.
- **[T20260510-7]** — Make leaf IDs unique across extractors and preserve every symbol across SQL and fallback paths.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
