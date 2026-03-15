# Remove ANTHROPIC_API_KEY from Claude Required Env Vars

**Goal:** Drop `ANTHROPIC_API_KEY` from Claude provider's hermetic env requirement; rely on HOME for credential resolution.
**Scope:** `orbit-agent/` provider definition and its tests; `orbit-core/` integration tests that inject a dummy key.
**Assumptions:** Claude Code resolves credentials from `~/.claude/` when HOME is set. No other code path gates on ANTHROPIC_API_KEY presence.
**Risks:** Low. HOME is already required. This is a strict reduction of env requirements.

## Task 1: Remove from provider definition

**Files:**
- Modify: `orbit-agent/src/providers/mod.rs`

Change:
```rust
// Before
AgentProvider::Claude => &["HOME", "PATH", "ANTHROPIC_API_KEY"],

// After
AgentProvider::Claude => &["HOME", "PATH"],
```

**Steps:**
1. Make the change.
2. `cargo build -p orbit-agent`

## Task 2: Update protocol_behavior tests

**Files:**
- Modify: `orbit-agent/tests/protocol_behavior.rs`

The test at line ~194 asserts `ANTHROPIC_API_KEY` is in `required_env_vars`.
Remove or invert that assertion — Claude should NOT require it.

**Steps:**
1. Remove the assertion that `ANTHROPIC_API_KEY must be in Claude required_env_vars`.
2. Optionally assert it is NOT present to make intent explicit.
3. `cargo test -p orbit-agent`

## Task 3: Update job_runtime_behavior integration tests

**Files:**
- Modify: `orbit-core/tests/job_runtime_behavior.rs`

Several tests (~line 868, 1368–1401):
- Remove `ANTHROPIC_API_KEY` from the hermetic pass list in test config strings.
- Remove `set_var("ANTHROPIC_API_KEY", "test-dummy-key")` setup / `remove_var` teardown.
- Adjust the agent-detection heuristic at ~line 868 if it branches on API key presence.

**Steps:**
1. Make changes.
2. `cargo test -p orbit-core`

## Final Verification
```bash
cargo test -p orbit-agent
cargo test -p orbit-core
cargo build --workspace
```