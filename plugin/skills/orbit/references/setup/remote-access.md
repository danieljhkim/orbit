# Remote access: dashboard and MCP

Reaching another machine's Orbit. Both surfaces are live access to *that
machine's* state — neither is synchronization, replication, or an offline copy.
The remote machine stays authoritative for every workspace and mutation it
serves.

## The dashboard

```bash
orbit web serve                    # loopback, port 7878, opens a browser
orbit web serve --port 8080 --no-open
```

One dashboard serves every workspace registered on that machine; requests select
one by ID, falling back to the default. It shows tasks, runs, routines, frictions,
and audit history.

To view a remote machine's dashboard, forward it over SSH:

```bash
orbit web connect <ssh-host>                    # anything ssh accepts, including a config alias
orbit web connect <ssh-host> --remote-port 7878 --port 9000
orbit web connect <ssh-host> --root /path/to/remote/workspace
```

`connect` first probes for a dashboard already running on the remote loopback. If
one answers, it attaches without touching that process. Otherwise it starts
`orbit web serve --no-open` over a second SSH connection and owns that process's
lifetime.

## The MCP surface

The same server, three transports:

```text
local:   client ──stdio──> orbit mcp serve ──> Orbit Core
remote:  client ──stdio──> ssh ──> orbit mcp serve ──> Orbit Core
socket:  client ──TCP────> orbit mcp listen ──> Orbit Core
```

**Local** is what `orbit mcp init` registers with an agent client — the client
launches `orbit mcp serve` itself over stdio.

**Remote** is direct SSH stdio. The local side starts a non-interactive SSH
process whose remote command is the same `orbit mcp serve`, and inherits stdin,
stdout, and stderr. It does not parse MCP frames, resolve a workspace, filter
tools, or make authorization decisions — it is a pipe.

**Socket** is for deployments that need one, typically reached through an SSH
tunnel:

```bash
orbit mcp listen                        # 127.0.0.1:7879 by default
orbit mcp listen 0.0.0.0:7879 --allow-non-loopback
```

Each accepted connection is an independent session against the same
server-local tool surface, resolved and audited exactly as a stdio session is.

## The security boundary — read this before exposing anything

**SSH is the access control. There is nothing else.**

- The dashboard refuses non-loopback binds and has no application
  authentication. Its Origin check is browser-CSRF mitigation, not access
  control. The Web API includes mutations, so anyone who reaches the forwarded
  port can act with the authority of the remote Orbit process.
- The MCP socket authenticates no client. It binds loopback for that reason, and
  `--allow-non-loopback` hands this machine's full tool surface to anyone who
  can reach the address. Pass it only where the network path is already
  restricted.

Neither surface has per-user permissions. If someone can reach the port, they
are an operator.

## Which machine is authoritative

The machine accepting `orbit mcp serve` or `orbit mcp listen` owns the call. It
derives its own process identity, resolves any required workspace against its own
registry, opens that server-local runtime, and records success, failure, or
denial at that boundary. A transport changes only how the bytes arrive.

Practical consequence: a workspace path in a tool call is resolved on the
*remote* machine. Passing a local path that doesn't exist there fails, and a path
that happens to exist on both resolves to the remote one.

## Workspace selection

Calls resolve a workspace from MCP initialize/session context or an explicit
`workspace` input. Do **not** add a global `--root` to `orbit mcp serve` to force
routing — the server rejects launch-root routing. If a call reports that
workspace selection is missing, supply the selector rather than changing the
launch command.
