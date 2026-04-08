# orbit-map

`orbit-map` is Orbit's default knowledge builder.

It scans a repository, builds structural knowledge artifacts, and writes deterministic outputs under `.orbit/knowledge/`. Those artifacts let Orbit work from a persistent, versioned understanding of the codebase instead of re-exploring the repo on every task.

“Transactional Code Execution System (TCES)”

## What It Produces

`orbit-map` writes:

```text
.orbit/knowledge/
  manifest.json
  graph/
    refs/current.json
    index/by-id.json
    objects/<prefix>/<sha256>.json
    blobs/<prefix>/<sha256>.txt
  files/
    <sha256>.json
```

These artifacts are intended to conform to Orbit's knowledge schema and support:

- lower token usage
- reproducible context
- faster task execution
- auditable, diff-friendly outputs

`orbit-map` implements the schema, but does not own it.

## Project Layout

```text
orbit_map/
  main.py
  service/
    graph_context.py
    bootstrap.py
  runtime/
    agent/
      base.py
      factory.py
      openai.py
      anthropic.py
      ollama.py
  graph/
    languages.py
    extraction/
      base.py
      registry.py
      python.py
  pipeline/
    __init__.py
    config.py
    context.py
    engine.py
    registry.py
    hash.py
    scan.py
    components/
      base.py
      scan.py
      hashes.py
      summarize.py
      architecture.py
      manifest.py
  schemas/
    knowledge.py
    graph/
      nodes.py
      contexts.py
      navigation.py
      locking.py
```

## Core Model

Instead of:

```text
LLM -> inspect repo -> answer -> discard context
```

Orbit uses:

```text
repo -> knowledge build -> knowledge artifacts -> Orbit runtime
```

That makes repo understanding a build artifact rather than an ephemeral side effect.

## Responsibilities

`orbit-map` is responsible for:

- scanning the repository
- hashing files for incremental rebuilds
- building a structural code graph
- writing schema-shaped knowledge artifacts

`orbit-map` is not responsible for:

- executing Orbit tasks
- making runtime workflow decisions
- owning the long-term Orbit agent runtime
- replacing language servers or full AST tooling

## CLI

Build the graph artifact:

```bash
orbit-map build graph
```

Run an incremental update:

```bash
orbit-map update graph
```

Choose a repo and output directory:

```bash
orbit-map build graph --repo . --output .orbit/knowledge
orbit-map build knowledge --repo . --output .orbit/knowledge
```

Enable verbose logging:

```bash
orbit-map --debug build graph
```

Render an agent-friendly knowledge bootstrap from existing graph and summary artifacts:

```bash
orbit-map knowledge bootstrap --output .orbit/knowledge
orbit-map knowledge bootstrap --output .orbit/knowledge --budget 8000
orbit-map knowledge bootstrap --output .orbit/knowledge --format json
```

Render a focused lineage pack from selected context nodes:

```bash
orbit-map knowledge pack file:src/a.py file:src/b.py
orbit-map knowledge pack leaf:src/a.py#a:function --depth 1 --siblings 2 --children 4
orbit-map knowledge pack dir:orbit-map/orbit_map/schemas --format json
```

Inspect the persisted graph:

```bash
orbit-map graph search PipelineContext --limit 5
orbit-map graph context file:src/a.py
orbit-map graph lineage file:src/a.py --include-self
orbit-map graph children dir:src
```

Build knowledge after the graph exists:

```bash
orbit-map build knowledge
```

If the graph is missing, `build knowledge` creates the graph first, then writes only missing file summary artifacts for that graph snapshot.

Today, selective knowledge updates are file-hash based from the graph snapshot. Function-level updates should key knowledge artifacts by `LeafNode.source_hash` or `source_blob_hash` so only changed leaves need to be regenerated.

## LLM Provider Selection

Some optional components use an LLM backend. Provider invocation lives behind
`orbit_map.runtime.agent`, which is a temporary Python runtime boundary shared
by knowledge-generation components and future Orbit-owned agent execution work.
Deterministic graph, bootstrap, context, and pack renderers should not import or
initialize provider implementations.

When optional LLM-backed components are included in the pipeline, the runtime
factory resolves its implementation from environment variables.

- `ORBIT_AGENT_PROVIDER`: `openai` (default), `anthropic`, or `ollama`
- `ORBIT_AGENT_MODEL`: override the provider's default model
- `OPENAI_API_KEY` / `OPENAI_BASE_URL`: used by `OpenAIAgent`
- `ANTHROPIC_API_KEY`: used by `AnthropicAgent`
- `OLLAMA_BASE_URL`: defaults to `http://localhost:11434`

### Runtime Boundary Plan

`orbit_map.runtime.agent` currently contains the provider interface and the
OpenAI, Anthropic, and Ollama adapters. The intended migration path is:

- keep deterministic graph/bootstrap/context commands provider-free
- keep summarization and architecture generation as explicit LLM-backed pipeline stages
- move provider configuration and adapters behind a reusable SDK/runtime package once Orbit's owned agent execution flow is ready
- make the future map builder consume that SDK/runtime instead of owning provider implementations

## Pipeline

The build runner executes an ordered list of components against a shared pipeline context.

Built-in component names:

- `scan_repo`
- `compute_hashes`
- `build_graph_dirs`
- `build_graph_files`
- `build_graph_leaves`
- `persist_graph`
- `manifest`
- `save_hash_cache`

Optional LLM-backed components that are available but not part of the default pipeline:

- `select_changed_paths`
- `summarize_files`
- `generate_architecture`

The default pipeline is:

```text
scan_repo
  -> compute_hashes
  -> build_graph_dirs
  -> build_graph_files
  -> build_graph_leaves
  -> persist_graph
  -> manifest
  -> save_hash_cache
```

## Swappable Components

Components are selected by name through a registry/config layer.

- `pipeline.registry.ComponentRegistry` maps names to component classes
- `pipeline.config.PipelineConfig` describes the ordered component list
- `pipeline.engine.run_build(...)` can accept explicit component instances, a `PipelineConfig`, or a custom registry

That means you can swap implementations without changing the runner itself, as long as the components cooperate through the shared `PipelineContext`.

## Programmatic Usage

You can configure the pipeline from Python with a registry and a named component list:

```python
from pathlib import Path

from orbit_map.pipeline.config import PipelineConfig
from orbit_map.pipeline.engine import run_build
from orbit_map.pipeline.registry import build_default_registry

repo_path = Path(".").resolve()
output_dir = repo_path / ".orbit" / "knowledge"

config = PipelineConfig.from_component_names(
    [
        "scan_repo",
        "compute_hashes",
        "build_graph_dirs",
        "build_graph_files",
        "build_graph_leaves",
        "persist_graph",
        "manifest",
        "save_hash_cache",
    ]
)

context = run_build(
    repo_path=repo_path,
    output_dir=output_dir,
    incremental=False,
    config=config,
    registry=build_default_registry(),
)
```

You can also register custom implementations:

```python
from orbit_map.pipeline.registry import build_default_registry

registry = build_default_registry()
registry.register(MyCustomSummarizeComponent)
```

Then reference the custom component by its `name` field inside `PipelineConfig`.

## Language Extractors

Directory and file graph construction is language-agnostic. Leaf extraction is delegated to language-specific extractors through `graph.extraction.GraphExtractorRegistry`.

The default extractor registry currently includes:

- `python`: uses the Python stdlib `ast` module to extract imports, exports, classes, functions, methods, signatures, source snippets, and stable leaf identities

Unsupported languages are still represented as `FileNode`s, but they do not get `LeafNode`s until an extractor is registered for their detected language.

To add another language, implement the extractor protocol:

```python
from orbit_map.graph.extraction import GraphExtractionInput, GraphExtractionResult


class RustGraphExtractor:
    language = "rust"

    def extract(self, input_data: GraphExtractionInput) -> GraphExtractionResult:
        ...
```

Then register it:

```python
from orbit_map.graph.extraction import build_default_extractor_registry
from orbit_map.pipeline.components.graph import BuildGraphLeavesComponent

extractor_registry = build_default_extractor_registry()
extractor_registry.register(RustGraphExtractor())

component = BuildGraphLeavesComponent(extractor_registry=extractor_registry)
```

## Graph Context Service

Agents should not need to read graph object files directly. The runtime-facing graph context service loads the persisted Merkle graph store and exposes navigable agent views:

```python
from orbit_map.service import GraphContextService

service = GraphContextService.from_knowledge_dir(".orbit/knowledge")

matches = service.search_nodes("PipelineContext", limit=5)
context = service.get_context(matches[0].id)
lineage = service.get_lineage(matches[0].id, include_self=True)
```

The service is backed by `GraphNavigator`, which can build:

- `DirContext`: subsystem-level directory context
- `FileContext`: file-level context with imports, exports, and top-level leaves when available
- `LeafContext`: editable symbol-level context with source, signatures, children, siblings, and history

## Knowledge Bootstrap

`orbit-map knowledge bootstrap` renders a deterministic whole-codebase briefing from the persisted graph and file summary artifacts. It does not invoke an LLM or any agent runtime.

The default markdown output is intentionally compact:

- repo stats
- directory/file lineage
- file summaries
- leaf names and signatures

Use `--detail` for the richer debug-oriented schema.

Source excerpts are opt-in and bounded by a separate budget.

## Lineage Packs

`orbit-map knowledge pack` renders a deterministic, context-specific graph summary for one or more selected nodes or selectors.

The pack accepts readable selectors such as:

- `dir:<path>`
- `file:<path>`
- `leaf:<path>#<symbol>:<kind>`

The output deduplicates shared ancestors, sibling context, and containing files so repeated context only appears once.
Use `--detail` if you want the richer debug-oriented schema instead of the compact default.

## Schema Notes

Important artifact types include:

- `manifest.json`: build metadata and artifact pointers
- `graph/refs/current.json`: Merkle graph entrypoint
- `graph/index/by-id.json`: stable node ID to graph object lookup
- `graph/objects/**/*.json`: content-addressed graph node/root objects
- `graph/blobs/**/*.txt`: content-addressed source blobs
- `files/*.json`: optional file summaries when summary components are enabled

Orbit is expected to validate and consume these artifacts separately.

## Design Principles

- deterministic where possible
- incremental by default
- schema-first
- auditable and diff-friendly
- replaceable pipeline stages

## Future Work

- alternate component implementations loaded from config files
- stronger dependency validation between pipeline stages
- richer retrieval artifacts
- more language extractors, starting with Rust
