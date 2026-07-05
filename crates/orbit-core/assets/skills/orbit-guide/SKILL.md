---
name: orbit-guide
description: Onboard a first-time user to Orbit when no `.orbit/` exists in the workspace — walk them through prereq verification, install path choice, MCP wiring, and the first task. Also covers Orbit feature-tour requests for users who have not committed yet. Triggers on phrasing like "set up orbit", "install orbit", "wire orbit into this repo", "what is orbit", "give me a tour", "I'm new to orbit", or any Orbit question asked from a workspace where `.orbit/` is missing. Defer to the `orbit` skill once the workspace is initialized and the user is operating inside the task lifecycle.
---

# Orbit Guide

## Purpose

Get a first-time user from zero to a usable Orbit workspace, then hand off to `orbit-create-task` for their first real task. Also handles feature-tour questions ("what is orbit", "what can it do", "give me a tour") by reading the canonical docs at invocation time rather than answering from memory.

## When to use this skill vs. `orbit`

- `.orbit/` missing in the current workspace → this skill.
- User asks "what is orbit", "give me a tour", or otherwise has not committed to using it yet → this skill.
- Workspace already initialized and the user references a task or asks to do Orbit work → defer to the `orbit` skill.

## Canonical sources

The README and config reference live in the Orbit repo. When the user is on a non-clone install path (plugin or binary-only install), there is no local copy yet — use `WebFetch` against the raw URLs below instead of answering from this skill's snapshot.

- Repo: https://github.com/danieljhkim/orbit
- Raw README: https://raw.githubusercontent.com/danieljhkim/orbit/main/README.md
- Raw config reference: https://raw.githubusercontent.com/danieljhkim/orbit/main/docs/CONFIG.md

Always re-fetch. The README is the source of truth for install commands, destructive-action confirmation rules, and prereq versions. This skill intentionally does not duplicate any of it.

## Step 1 — Detect current state

Run these in parallel; the answers determine which branch in Step 2 applies:

```bash
command -v orbit          # is the binary on PATH?
test -d .orbit            # is the workspace initialized?
test -d ~/.orbit          # is global state initialized?
rustc --version           # only relevant for the clone-and-build branch
```

Report the four results to the user before proposing any installs.

## Step 2 — Pick a setup path

Ask one question (per the user-interaction guardrails) and offer three branches. Whichever path the user picks, the actual commands come from the README — not from this skill.

1. **Clone-and-build branch (recommended).** README section "Setup via Agent Prompt". Read that block from the local clone (or via the raw README URL above) and follow it verbatim. It is the canonical agent-driven setup script.
2. **Plugin branch.** README sections "Claude Code Plugin vs CLI" and "Manual Setup".
3. **Curl-or-brew branch.** README section "Manual Setup".

Do not paraphrase the commands here. Re-read the README at invocation time; it may have moved on from this skill's snapshot.

## Step 3 — Run setup

Whichever branch was chosen in Step 2, follow the destructive-action confirmation rules stated in the README's "Setup via Agent Prompt" block. Those rules are the source of truth; this skill does not restate them. If the block has moved or changed, re-fetch the README rather than relying on this skill's snapshot.

If a step fails — missing toolchain, permission error, registration error — surface the failure to the user and offer `orbit-track-issues` to capture the friction. First-time setup is exactly the signal that skill exists for.

## Step 4 — Verify

After setup completes, confirm the install end-to-end:

```bash
orbit --version
orbit task list
orbit semantic stats   # only if the user opted into the semantic embedder
```

Report the output. If any of these fail, jump back to Step 3 — do not declare success.

## Step 5 — Hand off

Two handoff branches, depending on user intent:

- **First real task** — invoke `orbit-create-task`. Do not silently author the task; that skill enforces the task quality gates the user benefits from seeing.
- **Feature tour** ("what is orbit", "give me a tour") — read the README's `## Primary Features` section (locally or via the raw README URL above) and summarize against the user's stated goal, not generically. The tour content is the README's, not this skill's.

After hand-off, this skill's job is done. Subsequent Orbit work routes through the `orbit` skill and its lifecycle siblings.

## Routines & scheduling (`orbit sweep`)

Routines make Orbit the host's scheduler: a git-versioned YAML definition (`.orbit/routines/*.yaml`) pairs a cron trigger with a catalog `job:<name>` target, host pinning, and a retry/overlap policy. A stateless `orbit sweep` pass — invoked every minute by the OS clock (launchd / systemd timer) — fires whatever is due on this host as a normal run. Definitions sync across hosts via git; **all scheduler state (last fires, pauses, locks) is host-local in `~/.orbit/orbit.db` and never synced.**

**Canonical sources — read these at invocation time; don't answer routine field semantics from memory:**

- Design contract (schema, discovery, sweep, host-local state, clock): `docs/design/routines/` — start at `2_design.md` (§1 is the field-by-field schema, the source of truth).
- Command surface: `orbit routine --help` and `orbit sweep --help`.

Setup sequence (the *skeleton* is stable; re-read the design doc for full field semantics before hand-authoring a routine):

1. **Set this host's identity** — `orbit routine init --host-id <id>` writes `~/.orbit/host.toml`. The `<id>` is what a routine's `hosts:` list is matched against. Absent an explicit id it defaults to the machine hostname. Add `--install-clock` to also install and start the per-user OS clock unit that runs `orbit sweep` every minute (launchd agent on macOS, systemd user timer on Linux).
2. **Make a workspace a routine source** — add `[routines]` / `role = "source"` to that workspace's `.orbit/config.toml` (versioned; both hosts converge via `git pull`). The workspace must already be registered with Orbit. Only `role = "source"` is valid — any other value is a fail-closed config error.
3. **Add a routine** — drop a YAML file under `<source>/.orbit/routines/`. Minimal illustrative shape (see `2_design.md` §1 for every field and default):

   ```yaml
   schemaVersion: 1
   name: almanac-auto-commit          # unique across all sources on a host
   enabled: true                       # versioned kill-switch
   hosts: [dk-mac]                      # explicit pinning; no "any host" in v1
   trigger:
     cron: "0 22 * * *"                # 5-field, host-local time
     missed_run: skip                   # skip | catch_up_once (default: skip)
   target: job:almanac_commit_pipeline  # job:<name> only; activity: is rejected — wrap it in a one-step job
   policy:
     timeout_minutes: 10
     retries: { max: 2, backoff_minutes: 2 }
     overlap: forbid                    # forbid | allow
   ```

   Parsing is fail-closed: an invalid file (bad schema version, unknown field, unresolvable target, unparsable cron) makes *that routine* absent and reports a load error — it never fires with defaults.

4. **Verify without firing** — `orbit routine list` shows every routine with its toggle columns (enabled / pinned / paused) and computed next-due; `orbit sweep --dry-run` reports what *would* fire and records nothing. `orbit routine show <name>` adds recent fire history.

Observe & control:

- Fires are normal runs — they appear in `orbit run history` and carry the actor `routine/<name>`.
- `orbit routine pause <name>` / `resume <name>` suppress a routine on *this host only* (host-local, unversioned, durable across reboots).
- Toggle resolution order when a routine doesn't fire: `enabled: false` (versioned, everywhere) → host not in `hosts` (versioned, per host) → local pause (this host only). `orbit routine list` shows all three, so "why didn't this fire?" is one command.

If setup or a sweep misbehaves, surface it and offer `orbit-track-issues` to capture the friction.

## Cowork configuration

If the user runs the plugin inside **Cowork** (the Claude desktop app) and only the `orbit_graph_*` tools appear — no `orbit_task_add` / ADR / learning tools — it's the known workspace-discovery gap: Cowork launches the plugin's MCP server with cwd and `CLAUDE_PROJECT_DIR` set to an internal scratchpad, not the selected repo, so `serve` finds no `.orbit/` and falls back to the graph-only surface.

Fix: pass the repo explicitly via the global `--root` flag in the orbit MCP launch. The durable place is user config (`~/.claude/settings.json`), since the cached plugin copy is overwritten on reinstall. See the README "Cowork users" note for the exact `mcpServers` block — re-read it at invocation time rather than restating the args here. This is `--root` only; there is no env/cwd source for the repo path in Cowork today (see learning L-0065).

## Anti-patterns (DO NOT)

- Don't inline install commands, prereq versions, or destructive-action rules in this file. They rot independently from the README.
- Don't run the README setup block from memory. Read it at invocation time.
- Don't trigger on Orbit work once `.orbit/` exists — defer to `orbit`.
- Don't author the first task silently. Route through `orbit-create-task`.
