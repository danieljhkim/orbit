---
title: Install Orbit
description: "Install the Orbit CLI and initialize global and workspace-local state."
sidebar:
  order: 2
---

## Platform Support

Orbit's CLI runs on macOS, Linux, and Windows, but **OS-level sandbox enforcement of agent subprocesses is currently macOS only**, via `sandbox-exec`. The bundled `claude`, `codex`, and `gemini` executors declare `sandbox: macos-sandbox-exec` and require macOS to launch with a sandbox; on Linux and Windows the same activities run, but the spawned agent process is not wrapped in a kernel-level sandbox. Filesystem policies still apply to Orbit's own HTTP-tool builtins on every platform.

## Install

The recommended install is the install script:

```bash
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | sh
```

It detects your platform, downloads the matching release binary, and places it on your `PATH`.

### Alternatives

Homebrew (macOS, Linuxbrew):

```bash
brew install danieljhkim/tap/orbit
```

Claude Code plugin (skips the install script, downloads the binary on first MCP call):

```text
/plugin marketplace add danieljhkim/orbit
/plugin install orbit
```

The plugin registers Orbit's MCP server, skills, and orchestration subagents in Claude Code, and pulls the matching native `orbit` binary through the [`@orbit-tools/cli`](https://www.npmjs.com/package/@orbit-tools/cli) npm proxy on first invocation. Requires Node 18+ on `PATH`. To get the `orbit` CLI on your shell as well: `npm install -g @orbit-tools/cli`.

From source (requires Rust toolchain):

```bash
git clone https://github.com/danieljhkim/orbit.git
cd orbit
make install
```

### Pinned versions and custom install directory

```bash
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | ORBIT_VERSION=v0.1.0 sh
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | ORBIT_INSTALL_DIR="$HOME/.local/bin" sh
```

## Initialize State

Orbit has global state and workspace-local state.

```bash
orbit init
cd <repo>
orbit workspace init
```

`orbit init` seeds default skills under `~/.orbit/skills` and links them into `~/.agents/skills` and `~/.claude/skills`. Workspace skills are optional overrides by skill name.

Pass `--mcp` to also auto-detect and set up MCP client integrations during workspace initialization:

```bash
orbit workspace init --mcp
```

## Configure Orbit

`orbit init` seeds `~/.orbit/config.toml` and prompts for per-role agent settings (reviewer, implementer, planner). See [Configuration](../../reference/config/) for file locations, shape, and backend precedence.

## Update the Graph

`orbit workspace init` builds the initial repository graph automatically. Refresh it incrementally as the codebase changes:

```bash
orbit graph update
```

If the initial build fails during `orbit workspace init` (the command prints `graph build: failed (...), run \`orbit graph build\` manually`), retry it with:

```bash
orbit graph build
```
