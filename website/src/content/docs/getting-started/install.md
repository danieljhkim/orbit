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

It detects your platform, downloads the matching release binary, authenticates
the signed release checksums, validates the archive contents, and places it on
your `PATH`.

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

Codex plugin (also skips the install script):

```bash
codex plugin marketplace add danieljhkim/orbit --ref main
codex plugin add orbit@orbit
```

Both plugins register Orbit's MCP server and shared skills automatically. The
Claude package also includes its Claude-specific hooks and subagents. On the
first MCP call, either package pulls the matching native `orbit` binary through
the [`@orbit-tools/cli`](https://www.npmjs.com/package/@orbit-tools/cli) npm
proxy. Node 18+ must be on `PATH`. To get the `orbit` CLI on your shell as well,
run `npm install -g @orbit-tools/cli`.

Refresh an existing plugin installation after an Orbit release:

```text
# Claude Code
/plugin update orbit

# Codex CLI
codex plugin marketplace upgrade orbit
codex plugin add orbit@orbit
```

Start a fresh agent task from your repository and make a read-only MCP call to
confirm the installed package is active:

```bash
# Claude Code
claude -p 'Use the read-only orbit.task.list MCP tool to list up to three tasks, then report how many were returned.'

# Codex CLI
codex -C <repo> 'Use the read-only orbit.task.list MCP tool to list up to three tasks, then report how many were returned.'
```

From source (requires Rust toolchain):

```bash
git clone https://github.com/danieljhkim/orbit.git
cd orbit
make install
```

### Pinned versions and custom install directory

```bash
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | ORBIT_VERSION=v0.3.1 sh
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | ORBIT_INSTALL_DIR="$HOME/.local/bin" sh
```

`ORBIT_VERSION`, `ORBIT_INSTALL_REPO`, and `ORBIT_INSTALL_BASE_URL` change the
release source the installer trusts, so use them only for pinned releases,
forks, or controlled test mirrors. `ORBIT_INSTALL_BASE_URL` may use any
downloader-supported scheme, including `file://` for tests; the signature check
protects artifact integrity, not transport confidentiality.
`ORBIT_RELEASE_TRUSTED_KEYS_FILE` is the preferred override for the full
trusted signing-key set, including key IDs, `not_after`, and `revoked_at`; it
requires `ORBIT_RELEASE_TRUSTED_KEYS_FILE_ACKNOWLEDGE_TRUST_CHANGE=1` and should
be limited to tests or emergency operations.
`ORBIT_RELEASE_PUBLIC_KEY_FILE` is **deprecated** in favor of the trusted-keys
file (which is a strict superset); it still works for the single-key case and
requires `ORBIT_RELEASE_PUBLIC_KEY_FILE_ACKNOWLEDGE_TRUST_CHANGE=1`.

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

<!-- ORB-10117 -->
