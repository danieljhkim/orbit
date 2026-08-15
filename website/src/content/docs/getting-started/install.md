---
title: Install Orbit
description: "Install the Orbit CLI and initialize global and workspace-local state."
sidebar:
  order: 2
---

## Platform Support

Orbit's CLI runs on macOS, Linux, and Windows, but **OS-level sandbox
enforcement of agent subprocesses is currently macOS only**, via
`sandbox-exec`. The shipped `claude`, `codex`, `gemini`, and `grok` executor
assets declare the macOS sandbox; initialization drops that unsupported
primitive on other platforms so those activities run without a kernel-level
sandbox. Filesystem policies still apply to Orbit's own HTTP-tool builtins on
every platform.

## Install

The recommended project-agnostic install uses the npm binary proxy:

```bash
npm install -g @orbit-tools/cli
```

Node 18 or newer is required. The package downloads the matching native Orbit
binary and exposes it on your `PATH`.

### Alternatives

From a trusted source checkout, the release installer detects the platform,
downloads the matching release binary, authenticates signed checksums, validates
the archive contents, and installs the binary:

```bash
./install.sh
```

For development from a source checkout (requires the Rust toolchain):

```bash
make install
```

### Pinned versions and custom install directory

```bash
ORBIT_VERSION=v0.9.2 ./install.sh
ORBIT_INSTALL_DIR="$HOME/.local/bin" ./install.sh
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

`orbit init` seeds `~/.orbit/config.toml` with crews for detected provider CLIs.
See [Configuration](../../reference/config/) for file locations and shape.
