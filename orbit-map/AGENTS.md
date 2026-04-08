# AGENTS — orbit-map (Python)

This file provides context for agents working on the `orbit-map` Python CLI app.

---

## What is orbit-map?

`orbit-map` is the **default knowledge builder** for Orbit. It scans a codebase, extracts structure and semantics, and produces deterministic artifacts under `.orbit/knowledge/`.

The core idea: instead of agents re-exploring a repo every time, `orbit-map` **pre-compiles code understanding into persistent, versioned artifacts** that Orbit consumes at runtime. Think `cargo build` for code knowledge.

---

## Relationship to the Rust Codebase

The Orbit Rust workspace lives at `orbit/` and contains 10 crates:

```
orbit-types → orbit-policy, orbit-exec → orbit-tools → orbit-store, orbit-agent → orbit-engine → orbit-core → orbit-cli
```

The Rust `orbit-agent` crate (`orbit/orbit-agent/`) currently handles the **agent runtime** — provider abstraction (Claude, Codex, mock) via the `AgentRuntime` trait.

This Python `orbit-map` handles **knowledge building**. Its optional LLM-backed summarization stages use a temporary provider boundary under `orbit_map.runtime.agent`, but deterministic graph/bootstrap/context code should not import or initialize provider implementations. Long term, provider adapters should move behind a reusable Orbit SDK/runtime that both knowledge generation and Orbit-owned execution flows can consume.

---

## Pipeline

```
scan repo → hash files → build graph → write manifest
```

Planned module layout:

```
orbit-map/
  main.py
  pipeline/
    scan.py       # file walking
    hash.py       # content hashing (sha256)
    summarize.py  # optional file-level LLM summaries
    architecture.py  # optional system-level LLM summaries
  runtime/
    agent/        # temporary provider runtime boundary
  schemas/
    knowledge.py  # pydantic models for schema validation
```

Key libraries: `pathlib`, `hashlib`, `pydantic`, `tiktoken`, LLM client (OpenAI / local).

---

## Output: Knowledge Artifacts

All output is written to `.orbit/knowledge/` with this structure:

```
.orbit/knowledge/
  manifest.json       # entry point — schema version, timestamp, repo root
  graph/              # content-addressed graph objects, refs, indexes, blobs
  files/
    <hash>.json       # optional per-file summary, symbols, imports/exports
```

### Schema (v1) — defined in `schema.md`

**manifest.json**: `schemaVersion`, `generated_at`, `repo_root`, `artifacts` pointers.

**graph/refs/current.json**: current graph root object and schema metadata.

**files/\<hash\>.json**: `path`, `hash`, `language`, `summary`, `symbols` (name/kind/signature/description), `imports`, `exports`, `metadata` (size_bytes, last_modified).

Optional: `.orbit/cache/hashes.json` for incremental rebuild support.

---

## How Orbit Consumes Knowledge

Orbit never explores the codebase directly. At runtime:

1. Load `manifest.json` and validate schema version
2. Load the graph from `graph/refs/current.json`
3. Overlay file summaries from `files/<hash>.json` when present
4. Render deterministic bootstrap, pack, or graph context output within token budget

---

## CLI

```bash
orbit-map build graph            # build graph artifacts
orbit-map build knowledge        # build graph if needed, then file knowledge
orbit-map update graph           # incremental graph update
orbit-map update knowledge       # incremental knowledge update
orbit-map build graph --repo . --output .orbit/knowledge
```

---

## Design Principles

- **Deterministic** — no randomness in output
- **Incremental** — only process what changed
- **Auditable** — diff-friendly JSON
- **Minimal** — avoid over-modeling
- **Decoupled** — schema-first; builder is replaceable (any tool can produce compatible artifacts)

---

## What orbit-map Does NOT Do

- Execute tasks
- Make runtime decisions
- Interact with Orbit workflows directly
- Replace language servers or provide full AST fidelity

---

## Future Work

- Embedding-based retrieval
- Graph construction
- Cross-repo linking
- Symbol-level indexing
- Language-specific parsers (AST)
- Alternative/pluggable knowledge builders
