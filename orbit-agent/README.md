# orbit-agent

`orbit-agent` is Orbit's default knowledge builder.

It scans a repository, builds structural knowledge artifacts, and writes deterministic outputs under `.orbit/knowledge/`. Those artifacts let Orbit work from a persistent, versioned understanding of the codebase instead of re-exploring the repo on every task.

“Transactional Code Execution System (TCES)”

## What It Produces

`orbit-agent` writes:

```text
.orbit/knowledge/
  manifest.json
  graph.json
  files/
    <sha256>.json
```

These artifacts are intended to conform to Orbit's knowledge schema and support:

- lower token usage
- reproducible context
- faster task execution
- auditable, diff-friendly outputs

`orbit-agent` implements the schema, but does not own it.

## Project Layout

```text
orbit_agent/
  main.py
  service/
    graph_context.py
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

`orbit-agent` is responsible for:

- scanning the repository
- hashing files for incremental rebuilds
- building a structural code graph
- writing schema-shaped knowledge artifacts

`orbit-agent` is not responsible for:

- executing Orbit tasks
- making runtime workflow decisions
- replacing language servers or full AST tooling

## CLI

Build a full knowledge snapshot:

```bash
orbit-agent build
```

Run an incremental update:

```bash
orbit-agent update
```

Choose a repo and output directory:

```bash
orbit-agent build --repo . --output .orbit/knowledge
```

Enable verbose logging:

```bash
orbit-agent --debug build
```

List registered pipeline components:

```bash
orbit-agent list-components
```

Inspect the persisted graph:

```bash
orbit-agent graph search PipelineContext --limit 5
orbit-agent graph context file:orbit-agent/orbit_agent/pipeline/context.py
orbit-agent graph lineage file:orbit-agent/orbit_agent/pipeline/context.py --include-self
orbit-agent graph children dir:orbit-agent/orbit_agent/pipeline
```

Select an ordered component pipeline by name:

```bash
orbit-agent build \
  --components scan_repo,compute_hashes,build_graph_dirs,build_graph_files,build_graph_leaves,persist_graph,manifest,save_hash_cache
```

## LLM Provider Selection

Some optional components use an LLM backend. `orbit-agent` resolves its LLM implementation from environment variables when those components are included in the pipeline.

- `ORBIT_AGENT_PROVIDER`: `openai` (default), `anthropic`, or `ollama`
- `ORBIT_AGENT_MODEL`: override the provider's default model
- `OPENAI_API_KEY` / `OPENAI_BASE_URL`: used by `OpenAIAgent`
- `ANTHROPIC_API_KEY`: used by `AnthropicAgent`
- `OLLAMA_BASE_URL`: defaults to `http://localhost:11434`

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

from orbit_agent.pipeline.config import PipelineConfig
from orbit_agent.pipeline.engine import run_build
from orbit_agent.pipeline.registry import build_default_registry

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
from orbit_agent.pipeline.registry import build_default_registry

registry = build_default_registry()
registry.register(MyCustomSummarizeComponent)
```

Then reference the custom component by its `name` field inside `PipelineConfig`.

## Language Extractors

Directory and file graph construction is language-agnostic. Leaf extraction is delegated to language-specific extractors through `graph.extraction.GraphExtractorRegistry`.

The default extractor registry currently includes:

- `python`: uses the Python stdlib `ast` module to extract imports, exports, classes, functions, methods, signatures, source snippets, and history entries

Unsupported languages are still represented as `FileNode`s, but they do not get `LeafNode`s until an extractor is registered for their detected language.

To add another language, implement the extractor protocol:

```python
from orbit_agent.graph.extraction import GraphExtractionInput, GraphExtractionResult


class RustGraphExtractor:
    language = "rust"

    def extract(self, input_data: GraphExtractionInput) -> GraphExtractionResult:
        ...
```

Then register it:

```python
from orbit_agent.graph.extraction import build_default_extractor_registry
from orbit_agent.pipeline.components.graph import BuildGraphLeavesComponent

extractor_registry = build_default_extractor_registry()
extractor_registry.register(RustGraphExtractor())

component = BuildGraphLeavesComponent(extractor_registry=extractor_registry)
```

## Graph Context Service

Agents should not need to read `graph.json` directly. The runtime-facing graph context service loads the persisted graph and exposes navigable agent views:

```python
from orbit_agent.service import GraphContextService

service = GraphContextService.from_knowledge_dir(".orbit/knowledge")

matches = service.search_nodes("PipelineContext", limit=5)
context = service.get_context(matches[0].id)
lineage = service.get_lineage(matches[0].id, include_self=True)
```

The service is backed by `GraphNavigator`, which can build:

- `DirContext`: subsystem-level directory context
- `FileContext`: file-level context with imports, exports, and top-level leaves when available
- `LeafContext`: editable symbol-level context with source, signatures, children, siblings, and history

## Schema Notes

Important artifact types include:

- `manifest.json`: build metadata and artifact pointers
- `graph.json`: directory/file/leaf graph of the repository
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
