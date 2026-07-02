# Spec: SSH-Tunnel Connect

`orbit web connect <ssh-host>` establishes a foreground SSH tunnel to a remote machine's loopback dashboard, guarantees the local endpoint answers `/healthz` before returning control, and guarantees the remote `orbit web serve` process is reaped when the tunnel ends. It adds no authentication of its own.

## Why This Exists

The dashboard binds loopback-only ([ORB-00360]) because it is unauthenticated. Viewing it from another machine therefore requires an authenticated tunnel. Done by hand (`ssh -L 7878:localhost:7878 host "orbit web serve --no-open"`) this leaks orphan remote processes on disconnect and gives no readiness signal. This spec is the contract `connect` upholds so the automated path is safe.

## Invariants

- **Loopback only, both ends.** The remote serve binds loopback; the forwarded local port binds loopback. `connect` never binds or requests a routable interface.
- **No Orbit auth.** Authentication and encryption are SSH's. `connect` adds no token, ACL, or session.
- **Remote never opens a browser.** `remote_serve_command` always includes `--no-open`; only the local side may open a browser (suppressed by local `--no-open`).
- **`--root` is shell-quoted.** Any value forwarded to the remote shell that may contain spaces is POSIX single-quoted.
- **`--global` / `--root` are passthrough only.** They change the remote serve's workspace scope, never the tunnel's security posture.
- **No orphan remote process.** The tunnel uses a pty (`ssh -tt`); dropping the local `ssh` delivers SIGHUP to the remote session, stopping the remote serve. `SshTunnel::Drop` enforces local teardown (SIGTERM then SIGKILL after a grace period) on every exit path.
- **Fail fast on forward failure.** `ExitOnForwardFailure=yes` — if the local port cannot be forwarded, `ssh` exits rather than running the remote command with a dead tunnel.

## Failure Modes

- **`orbit` not on remote PATH** → remote shell exits `127` → actionable error naming the non-interactive-PATH cause.
- **SSH cannot connect** (bad host, auth, network) → `ssh` exits `255` → actionable error.
- **Remote serve exits before ready** → classified by exit code; readiness loop returns the error rather than hanging.
- **Readiness timeout** (default 30s covering SSH connect + remote spawn) → explicit timeout error against `http://localhost:<port>/healthz`.
- **Local port race (TOCTOU)** → `select_local_port` probes then hands the port to `ssh`; if another process claims it in between, `ssh -L` fails loudly on startup. Acceptable for a developer convenience command.

## Migration / Compatibility

- `connect` dispatches in `main.rs` **before** eager workspace init, so it runs from any directory (its workspace is remote). Adding a `web` subcommand that needs no local runtime must extend that early-dispatch arm, not rely on the post-init path.

## Agent Signature

Authored by claude for [ORB-00029] (initial command + `--global` passthrough), 2026-07.
