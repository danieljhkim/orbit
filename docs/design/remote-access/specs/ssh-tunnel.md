# Spec: SSH-Tunnel Connect

`orbit web connect <ssh-host>` establishes a foreground SSH tunnel to a remote machine's loopback dashboard, attaching to an already-running remote dashboard when one answers and spawning one only when nothing does ([ORB-10708], [orbit web connect attaches to an already-running remote dashboard instead of always spawning one](../4_decisions.md#orbit-web-connect-attaches-to-an-already-running-remote-dashboard-instead-of-always-spawning-one)). It guarantees the local endpoint answers `/healthz` before returning control, and guarantees a remote `orbit web serve` process this invocation spawned is reaped when the tunnel ends — while an attached, pre-existing remote process is never touched. It adds no authentication of its own.

## Why This Exists

The dashboard binds loopback-only ([ORB-00360]) because it is unauthenticated. Viewing it from another machine therefore requires an authenticated tunnel. Done by hand (`ssh -L 7878:localhost:7878 host "orbit web serve --no-open"`) this leaks orphan remote processes on disconnect, gives no readiness signal, and cannot share an already-running remote server — a second manual tunnel just fails to bind. This spec is the contract `connect` upholds so the automated path is safe and reusable.

## Where It Lives

The mechanism is [`orbit_common::utility::ssh_tunnel`](../../../../crates/orbit-common/src/utility/ssh_tunnel.rs) ([ORB-10710], [Own the SSH local-forward tunnel once, at the leaf, shared by every loopback listener](../../mcp-bridge/4_decisions.md#own-the-ssh-local-forward-tunnel-once-at-the-leaf-shared-by-every-loopback-listener)): RAII child and teardown, port selection, forward arguments, `shell_quote`, `ssh` exit classification, readiness polling, and the attach-first `establish` sequence. Consumers supply a `TunnelSpec` (remote command, description, timeouts) and a readiness closure. `orbit web connect` and `orbit mcp serve --mode remote` are the two consumers; the invariants below bind both, with `/healthz` standing in for whatever readiness the consumer probes.

## Invariants

- **Loopback only, both ends.** The remote serve binds loopback; the forwarded local port binds loopback. `connect` never binds or requests a routable interface.
- **No Orbit auth.** Authentication and encryption are SSH's. `connect` adds no token, ACL, or session.
- **Attach before spawn.** `establish` always opens a bare `-N` forward (no remote command) and polls `/healthz` through it before deciding anything. Readiness is decided by the `/healthz` response, never by `ssh`'s own exit code, because a forward can be healthy while nothing is listening behind it. Only a probe timeout with no answer triggers spawn mode.
- **A spawn never happens behind an already-answering forward.** If the attach probe gets a `200`, no remote command is ever sent for that session — this is what keeps a second `connect` against a live remote dashboard from starting (or failing to start) a second one.
- **Remote never opens a browser.** the dashboard's `remote_serve_command` always includes `--no-open`; only the local side may open a browser (suppressed by local `--no-open`).
- **`--root` is shell-quoted.** Any value forwarded to the remote shell that may contain spaces is POSIX single-quoted.
- **`--global` / `--root` are passthrough only.** They change the remote serve's workspace scope, never the tunnel's security posture. They apply only in spawn mode — attach mode sends no remote command at all, so they have no effect on an attached session.
- **No orphan spawned process; no touching an attached one.** In spawn mode the tunnel uses a pty (`ssh -tt`); dropping the local `ssh` delivers SIGHUP to the remote session, stopping the remote serve this invocation started. In attach mode the tunnel carries no remote command, so dropping it only closes the forward — a pre-existing remote process is never signalled. `SshTunnel::Drop` enforces local teardown (SIGTERM then SIGKILL after a grace period) on every exit path in both modes.
- **Fail fast on forward failure.** `ExitOnForwardFailure=yes` on both the probe and the spawn-mode tunnel — if the local port cannot be forwarded, `ssh` exits rather than running (or waiting to run) a remote command with a dead tunnel.

## Failure Modes

- **`orbit` not on remote PATH** (spawn mode) → remote shell exits `127` → actionable error naming the non-interactive-PATH cause.
- **SSH cannot connect** (bad host, auth, network) → `ssh` exits `255` → actionable error. Surfaces identically whether it happens during the attach probe or the spawn-mode wait.
- **Remote serve exits before ready** (spawn mode) → classified by exit code; readiness loop returns the error rather than hanging.
- **Attach probe timeout** (`ATTACH_PROBE_TIMEOUT`, 5s) → not an error: nothing answered, so `connect` tears down the probe forward and falls back to spawn mode.
- **Readiness timeout** (spawn mode; default 30s covering SSH connect + remote spawn) → explicit timeout error against `http://localhost:<port>/healthz`.
- **Local port race (TOCTOU)** → `ssh_tunnel::select_local_port` probes then hands the port to `ssh`; if another process claims it in between, `ssh -L` fails loudly on startup. Acceptable for a developer convenience command.
- **Attach-vs-spawn race (TOCTOU)** → two `connect` invocations starting within the same probe window can both see nothing answering and both fall back to spawning; the second spawn fails to bind the remote port and surfaces via the same exit-code classification as any other spawn-mode bind failure. Not eliminated by this spec; accepted for a developer convenience command.

## Migration / Compatibility

- `connect` is declared `RuntimeNeed::Forbidden` in the exhaustive command-operation registry, so `main.rs` dispatches it before eager workspace init and it runs from any directory (its workspace is remote). Any new runtime-free `web` subcommand must declare that policy in the same `Commands::operation` arm; do not recreate an early-dispatch match in `main.rs`.

## Agent Signature

Authored by claude for [ORB-00029] (initial command + `--global` passthrough), 2026-07. Extended by claude for [ORB-10708] (attach-before-spawn probing), 2026-08.
