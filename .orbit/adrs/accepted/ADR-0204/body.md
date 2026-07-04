## Context

Something must wake the scheduler. Alternatives: a resident `orbit schedulerd` owning timers in-process (sub-minute precision, event triggers, but a daemon to supervise on two platforms), or delegating wake-ups to the OS schedulers that already exist (launchd on macOS, systemd timers on Linux) invoking a stateless pass every minute.

## Decision

launchd (`StartInterval` 60s) and a systemd timer (`OnCalendar=*:*:00`, `Persistent=true`) invoke `orbit sweep` every minute; sweep is stateless-in, durable-out. Missed-fire semantics split between the OS layer (wake/persistence behavior) and per-routine `missed_run` policy. There is no resident Orbit daemon.

## Consequences

- No process supervision, crash recovery, or memory-leak surface; a wedged pass affects one minute, not the scheduler.
- launchd wake behavior and `Persistent=true` pair with `missed_run: catch_up_once` to cover laptop sleep and host downtime.
- Cost: minute granularity is a hard floor and event triggers are structurally impossible in v1; correct behavior depends on two platform-specific unit files that must be kept in parity and tested on both platforms.