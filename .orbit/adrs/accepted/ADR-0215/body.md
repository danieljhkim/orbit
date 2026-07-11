## Context
ORB-10129 ships the triage pipeline as a default, but routines have no global directory: discovery reads `.orbit/routines/*.yaml` from `[routines] role = "source"` workspaces, v1 requires explicit host pinning (no "any host"), and routine names must be unique across all sources on a host — so a static shipped YAML cannot work. The real alternatives were leaving the triage routine workspace-authored (no out-of-the-box self-healing) or adding a global routines directory (a discovery-model change ADR-0205 deliberately avoided).

## Decision
`orbit init` (workspace branch) seeds `DEFAULT_ROUTINE_FILES` templates into `.orbit/routines/`, resolving `__ORBIT_HOST_ID__` via `resolve_host_id` and `__ORBIT_ROUTINE_NAME__` from a workspace-directory slug (`task-triage-<workspace>`), validating each rendered document fail-closed before writing. Plain re-init preserves user edits; `--refresh-defaults`/`--force` re-render. Seeded routines are inert until the workspace opts into `[routines] role = "source"`.

## Consequences
- A fresh workspace gets working periodic triage (hourly, `overlap: forbid`, sparser than the ~20-minute ship sweep) the moment it becomes a routine source — no bespoke per-box scripting.
- Per-workspace names let multiple seeded source workspaces coexist on one host despite the global name-uniqueness rule.
- The seeded file pins the initializing host; sharing the repo to another host needs a re-init with `--refresh-defaults` or a hand edit of `hosts:`.
- Cost: `orbit init` output now depends on the machine it runs on (host id, directory name) — two clones of the same repo can carry differently-rendered routine files, and identical workspace directory names on one host still collide fail-closed.