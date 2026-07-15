## Context
ORB-10129 ships the triage pipeline as a default, but routines have no global directory: discovery reads `.orbit/routines/*.yaml` from `[routines] role = "source"` workspaces, v1 requires explicit host pinning (no "any host"), and routine names must be unique across all sources on a host — so a static shipped YAML cannot work. The real alternatives were leaving defaults workspace-authored from scratch or adding a global routines directory (a discovery-model change ADR-0205 deliberately avoided).

## Decision
`orbit init` (workspace branch) seeds `DEFAULT_ROUTINE_FILES` templates into `.orbit/routines/`, resolving `__ORBIT_HOST_ID__` via `resolve_host_id` and `__ORBIT_ROUTINE_NAME__` from a workspace-directory slug, validating each rendered document fail-closed before writing. Every default is disabled. Plain re-init creates missing defaults while preserving existing definitions byte-for-byte; destructive `--force` recreates templates. A routine fires only after the workspace is a routine source and its versioned `enabled` field is set true.

## Consequences
- Fresh workspaces get reviewable routine definitions without silently granting scheduled execution.
- Per-workspace names let multiple seeded source workspaces coexist on one host despite the global name-uniqueness rule.
- The seeded file pins the initializing host; sharing the repo to another host needs a hand edit of `hosts:` or recreation during destructive initialization.
- Cost: `orbit init` output depends on the machine it runs on (host id, directory name), and routine template improvements do not overwrite existing workspace-authored files.