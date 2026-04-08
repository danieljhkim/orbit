# Orbit: GEMINI.md

## Project Overview
Orbit is a local-first workflow engine that coordinates humans and AI agents (e.g., Claude, Codex) directly within a repository. It operates on a "Minimize tokens, Maximize determinism" philosophy, offloading repetitive decisions to the engine so agents can focus on intent and design.

### Core Architecture (Rust Workspace)
The project is structured as a layered set of Rust crates:
- **`orbit-cli`**: The entry point (`orbit` binary), handling command dispatch and output.
- **`orbit-core`**: Runtime bootstrap, config layering, and asset seeding.
- **`orbit-engine`**: Activity and job execution, template rendering, and retry logic.
- **`orbit-agent`**: Provider abstractions (Claude, Codex) and agent-facing interfaces.
- **`orbit-store`**: Persistence layer using YAML (for artifacts) and SQLite (for audit logs).
- **`orbit-types`**: Shared domain types and error definitions (the "leaf" dependency).
- **`orbit-tools`**: Built-in tool registry (fs, git, github, etc.).
- **`orbit-policy`**: RBAC and decision engine.

### Knowledge Graph (Python)
A secondary Python component (`/orbit-agent`) manages codebase "knowledge" by scanning the repository, building a code graph, and generating deterministic lineage packs for agent context.

---

## Building and Running

### Prerequisites
- Rust (latest stable)
- Python 3.11+
- GitHub CLI (`gh`) - required for PR-based workflows (`ship`)

### Key Commands
The project uses a `Makefile` to wrap common `cargo` operations:

- **Build**: `make build` (debug) or `make release` (optimized)
- **Install**: `make install` (installs the `orbit` binary to your cargo bin path)
- **Test**: `make test` (runs workspace-wide tests)
- **Lint/Format**: `make clippy` and `make fmt`
- **Check**: `make check`
- **CI Pass**: `make ci` (runs fmt, clippy, and test)

### Local Execution
To run the CLI directly from source:
```bash
cargo run -p orbit-cli -- <args>
# or using make
make run ARGS="task list"
```

---

## Development Conventions

### State & Scoping
- **Global Root**: `~/.orbit/` (initialized via `orbit init`).
- **Workspace Root**: `.orbit/` within a project repo (initialized via `orbit workspace init`).
- **Persistence**: Tasks and Job definitions are stored as YAML in `.orbit/` for git-friendliness. Audit logs are stored in SQLite (`.orbit/audit.db`).

### Mental Model for Contributions
- **Tasks**: Units of work with a lifecycle (Proposed -> Backlog -> In Progress -> Review -> Done).
- **Activities**: Atomic operations defined in YAML with strict JSON Schema input/output contracts.
- **Jobs**: Composable workflows defined in YAML that sequence activities.
- **Workflows**: User-facing aliases (`ship`, `review`) over specific jobs.

### Dependency Rules
- **Layering**: Strictly follow the dependency graph (Types -> Store/Policy/Exec -> Tools -> Agent -> Engine -> Core -> CLI). Lower layers must never depend on higher layers.
- **Error Handling**: Use `OrbitError` from `orbit-types` for all workspace-wide errors.

### Tooling
- **Tracing**: Use the `tracing` crate for logging. CLI output defaults to `warn`, use `--debug` to increase verbosity.
- **Agents**: Agents should interact with Orbit via the `orbit tool run` surface rather than the human-oriented CLI commands where possible.
