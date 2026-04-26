---
title: Install Orbit
description: "Install the Orbit CLI and initialize global and workspace-local state."
sidebar:
  order: 2
---

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

Pass `--no-mcp` if you want workspace initialization without MCP client setup:

```bash
orbit workspace init --no-mcp
```

## Build the Graph

Build the initial repository graph before asking agents to reason over code structure.

```bash
orbit graph build
```

You can later update it incrementally:

```bash
orbit graph update
```
