## Context
The OS sweep clock is shared host infrastructure but previously had a hard-coded minutely cadence and only native-manager controls. The alternatives were a workspace routine setting, which would make one workspace own host infrastructure, or a host-local configuration plus Orbit CLI controls.

## Decision
Store the supported whole-minute cadence in host-local `~/.orbit/clock.toml` and expose it through `orbit routine clock`. Native launchd/systemd user services remain the authority for enabled state; routine pauses and manual `orbit sweep` remain separate.

## Consequences
- Clock status reports configured and effective cadence, and native-manager failures include recovery commands.
- Cost: the host-local setting intentionally does not travel with a workspace, so operators configure each host separately.