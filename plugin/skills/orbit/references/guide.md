# Orbit Guide (first-time setup & tour)

Get a first-time user from zero to a usable Orbit workspace, then hand off to `orbit-task` for their first real task. Also handles feature-tour questions ("what is orbit", "what can it do", "give me a tour") by reading the canonical docs at invocation time rather than answering from memory.

## When to use this reference vs. the rest of `orbit`

- `.orbit/` missing in the current workspace → this reference.
- User asks "what is orbit", "give me a tour", or otherwise has not committed to using it yet → this reference.
- Workspace already initialized and the user references a task or asks to do Orbit work → the rest of the `orbit` skill and its siblings.

## Canonical sources

The README and config reference live in the Orbit repo. On a non-clone install path (plugin or binary-only install) there is no local copy yet — use `WebFetch` against the raw URLs below instead of answering from this snapshot.

- Repo: https://github.com/danieljhkim/orbit
- Raw README: https://raw.githubusercontent.com/danieljhkim/orbit/main/README.md
- Raw config reference: https://raw.githubusercontent.com/danieljhkim/orbit/main/docs/CONFIG.md

Always re-fetch. The README is the source of truth for install commands, destructive-action confirmation rules, and prereq versions. This reference intentionally does not duplicate any of it.

## Step 1 — Detect current state

Run in parallel; the answers determine which branch in Step 2 applies:

```bash
command -v orbit          # is the binary on PATH?
test -d .orbit            # is the workspace initialized?
test -d ~/.orbit          # is global state initialized?
rustc --version           # only relevant for the clone-and-build branch
```

Report the four results to the user before proposing any installs.

## Step 2 — Pick a setup path

Ask one question (per the user-interaction guardrails) and offer three branches. The actual commands come from the README, not from this reference:

1. **Clone-and-build branch (recommended).** README section "Setup via Agent Prompt" — the canonical agent-driven setup script.
2. **Plugin branch.** README sections "Claude Code Plugin vs CLI" and "Manual Setup".
3. **Curl-or-brew branch.** README section "Manual Setup".

Do not paraphrase the commands here. Re-read the README at invocation time; it may have moved on from this snapshot.

## Step 3 — Run setup

Follow the destructive-action confirmation rules stated in the README's "Setup via Agent Prompt" block — that is the source of truth. If a step fails (missing toolchain, permission error, registration error), surface the failure and offer `orbit-task` friction reporting to capture it. First-time setup is exactly the signal that exists for.

## Step 4 — Verify

```bash
orbit --version
orbit task list
orbit semantic stats   # only if the user opted into the semantic embedder
```

Report the output. If any of these fail, jump back to Step 3 — do not declare success.

## Step 5 — Hand off

- **First real task** — invoke `orbit-task`'s create workflow. Do not silently author the task; it enforces quality gates the user benefits from seeing.
- **Feature tour** — read the README's `## Primary Features` section and summarize against the user's stated goal, not generically. The tour content is the README's, not this reference's.

After hand-off, subsequent Orbit work routes through the rest of the `orbit` skill and its lifecycle siblings.

## Routines & scheduling (`orbit sweep`)

Routines make Orbit the host's scheduler: a git-versioned YAML definition (`.orbit/routines/*.yaml`) pairs a cron trigger with a catalog `job:<name>` target, host pinning, and a retry/overlap policy. A stateless `orbit sweep` pass — invoked every minute by the OS clock (launchd / systemd timer) — fires whatever is due on this host as a normal run. Definitions sync across hosts via git; **all scheduler state (last fires, pauses, locks) is host-local in `~/.orbit/orbit.db` and never synced.** Full detail (job/activity/`orbit run` usage) lives in the `orbit-workflow` skill; this section covers setup only.

**Canonical sources — read at invocation time; don't answer routine field semantics from memory:** command surface via `orbit routine --help` and `orbit sweep --help` (field-by-field schema).

Setup sequence (skeleton is stable; re-read the design doc for full field semantics before hand-authoring a routine):

1. **Set this host's identity** — `orbit routine init --host-id <id>` writes `~/.orbit/host.toml` (defaults to hostname). Add `--install-clock` to also install and start the per-user OS clock unit that runs `orbit sweep` every minute.
2. **Make a workspace a routine source** — add `[routines]` / `role = "source"` to that workspace's `.orbit/config.toml`. The workspace must already be registered with Orbit; any other `role` value is a fail-closed config error.
3. **Add a routine** — drop a YAML file under `<source>/.orbit/routines/`:

   ```yaml
   schemaVersion: 1
   name: <routine-name>               # unique across all sources on a host
   enabled: true                       # versioned kill-switch
   hosts: [<host-id>]                   # explicit pinning; no "any host" in v1
   trigger:
     cron: "0 22 * * *"                # 5-field, host-local time
     missed_run: skip                   # skip | catch_up_once (default: skip)
   target: job:<job-name>               # job:<name> only; activity: is rejected — wrap it in a one-step job
   policy:
     timeout_minutes: 10
     retries: { max: 2, backoff_minutes: 2 }
     overlap: forbid                    # forbid | allow
   ```

   Parsing is fail-closed: an invalid file (bad schema version, unknown field, unresolvable target, unparsable cron) makes *that routine* absent and reports a load error — it never fires with defaults.

4. **Verify without firing** — `orbit routine list` shows every routine with toggle columns (enabled / pinned / paused) and computed next-due; `orbit sweep --dry-run` reports what *would* fire and records nothing. `orbit routine show <name>` adds recent fire history.

Observe & control:

- Fires are normal runs — they appear in `orbit run history` and carry the actor `routine/<name>`.
- `orbit routine pause <name>` / `resume <name>` suppress a routine on *this host only* (host-local, unversioned, durable across reboots).
- Toggle resolution order when a routine doesn't fire: `enabled: false` (versioned, everywhere) → host not in `hosts` (versioned, per host) → local pause (this host only). `orbit routine list` shows all three, so "why didn't this fire?" is one command.

If setup or a sweep misbehaves, surface it and offer `orbit-task` friction reporting.

## Cowork configuration

Orbit advertises the canonical MCP surface independently of Cowork's launch cwd or
`CLAUDE_PROJECT_DIR`. Do not add the global `--root` flag to `orbit mcp serve`; the
server rejects launch-root routing. Calls resolve a registered workspace from MCP
initialize/session context or an explicit `workspace` input. If a call reports that
workspace selection is missing, provide that selector rather than changing the MCP
launch command.

## Anti-patterns (DO NOT)

- Don't inline install commands, prereq versions, or destructive-action rules in this file. They rot independently from the README.
- Don't run the README setup block from memory. Read it at invocation time.
- Don't trigger this reference once `.orbit/` exists — use the rest of the `orbit` skill.
- Don't author the first task silently. Route through `orbit-task`'s create workflow.
