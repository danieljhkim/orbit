# Setting Orbit up

Get from nothing to a workspace where tasks, search, and dispatch all work. Use
this when `.orbit/` is absent, when the user is still deciding whether to adopt
Orbit, or when a second machine or repository needs onboarding.

Install commands are deliberately **not** duplicated here — they rot
independently. Read them at invocation time from the project README:

- Repo: https://github.com/danieljhkim/orbit
- Raw README: https://raw.githubusercontent.com/danieljhkim/orbit/main/README.md
- Raw config reference: https://raw.githubusercontent.com/danieljhkim/orbit/main/docs/CONFIG.md

The README is the source of truth for install commands, prerequisite versions,
destructive-action confirmation rules, and the Linux sandbox host prerequisite.
Fetch it; do not answer from memory.

## Step 1 — Detect current state

Run in parallel; the answers pick the branch:

```bash
command -v orbit          # is the binary on PATH?
test -d .orbit            # is this workspace initialized?
test -d ~/.orbit          # is this machine initialized?
```

Report all three before proposing any install.

## Step 2 — Install

Three paths, all documented in the README: clone-and-build, the plugin package,
or a prebuilt binary. Ask which the user wants rather than assuming. Follow the
README's destructive-action confirmation rules exactly.

## Step 3 — Initialize the machine

```bash
orbit init
```

This creates `~/.orbit/` and, on a fresh machine, establishes host identity. Two
values are asked for and both matter:

**Host name** — how this machine is named in routine `hosts:` pins. Defaults to
the OS hostname. Renameable later with `orbit host rename`.

**Task prefix** — 2–5 uppercase ASCII letters that namespace every task ID this
machine allocates. **Chosen once and never changed.** Its whole purpose is to
keep two machines that share a repository from allocating the same ID, so on a
second machine pick a *different* prefix — see [multi-host.md](multi-host.md)
before initializing one.

Non-interactive runs must pass both explicitly:

```bash
orbit init --non-interactive --host-name <name> --task-prefix <PREFIX>
```

## Step 4 — Initialize the workspace

From the repository root:

```bash
orbit workspace init --mcp
```

This registers the checkout, creates `.orbit/`, and seeds the default skills,
jobs, activities, policies, routines, and auto-task definitions. `--mcp`
auto-detects installed agent clients and registers Orbit's MCP server with them.

Two things to know about what it seeded:

- **Every routine and auto-task ships disabled.** The automation layer is
  installed but dark until someone reviews and opts in. Do not assume a fresh
  workspace schedules anything. → [automation.md](automation.md)
- **Definitions belong in git; state does not.** `orbit workspace init` seeds a
  `.gitignore` pattern that ignores `.orbit/` and then re-includes the versioned
  definition directories. Keep it.

Targeting specific clients, or a second pass later:

```bash
orbit mcp init --client claude --client codex        # repeatable
orbit mcp init --all --scope home                    # user-level rather than repo-local
```

Supported clients: `claude`, `codex`, `gemini`, `grok`, `cursor`, `vscode`,
`windsurf`. `--scope workspace` (the default) writes repo-local config;
`--scope home` writes user-level config.

## Step 5 — Verify

```bash
orbit --version
orbit workspace show
orbit task list
orbit doctor
```

`orbit doctor` is the real check — it inspects config, database, disk, indexes,
locks, and runs. Report its output. If anything fails, fix it before handing
off; do not declare success on a passing `--version` alone.

Optional, and only with explicit operator consent — semantic search needs a
local companion downloaded:

```bash
orbit semantic install
orbit semantic stats
```

Lexical search works without it. → [search.md](../search.md)

## Step 6 — Hand off

- **First real task** — route through [task-authoring.md](../task-authoring.md).
  Do not silently author it; the quality gates are worth the user seeing.
- **Feature tour** — read the README's feature section and summarize against the
  user's stated goal, not generically.

## What to set up next

In rough order of payoff, once the first task has landed:

1. **A docs corpus** — register the markdown the repo already has, so agents
   retrieve by concept instead of filename. → [docs-corpus.md](../docs-corpus.md)
2. **Crews and base branch** — point `workflow.base_branch` at the branch task
   PRs should target, and set a default crew. → [configuration.md](configuration.md)
3. **The scheduler** — host clock, then routines, in the documented order.
   → [automation.md](automation.md)
4. **Recurring chores** — QA sweeps, friction curation, anything periodic.
   → [auto-tasks.md](auto-tasks.md)

## Anti-patterns

- Don't inline install commands, prerequisite versions, or Linux sandbox
  remediation here. Read the README at invocation time.
- Don't pick a task prefix casually on a second machine. It cannot be changed.
- Don't enable every seeded routine at once. Enable worktree GC before, not
  after, scheduling ship traffic.
- Don't run `orbit semantic install` without consent — it downloads a model.
