## Context
ADR-006 left CLI backends outside Orbit's tool-layer enforcement: the harness emits `tool_allowlist.harness_delegated`, but Claude/Codex/Gemini built-in tools run with the orbit process's filesystem rights. Provider-native sandboxes were inconsistent (`codex --sandbox`, `gemini -s`, no Claude equivalent), leaving `fsProfile` unenforced for some CLI runs.

## Decision
Add `orbit-exec::macos_sandbox` as the declarative seam: compile a `ResolvedFsProfile` to SBPL and wrap Claude, Codex, and Gemini invocations with `sandbox-exec -f <profile>` when executor YAML declares `spec.sandbox: macos-sandbox-exec`. When Orbit owns the outer sandbox, neutralize provider-native sandbox flags so there is one filesystem authority. Resolve descriptors in `V2RuntimeHost::resolve_executor_sandbox` and compile SBPL in orbit-engine near the spawn site.

## Consequences
- All three providers share `FsProfile` compiled to SBPL as the macOS filesystem authority, giving Claude OS-enforced narrowing too.
- `allow_fallback` can degrade gracefully, but the safe default is fail-closed; Linux, Docker, network restriction, and activity-level overrides stay out of scope for v1.
- Cost: SBPL writes are static text; complex `denyRead` / `denyModify` rule combinations don't always translate cleanly. Simple subtree denials use `subpath`; non-subpath deny globs use SBPL `regex` to avoid over-denying the containing directory. Activities that need precise allow-side glob semantics under sandbox should declare profiles with explicit subpath roots.
