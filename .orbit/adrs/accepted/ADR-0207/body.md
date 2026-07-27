## Context

Some recurring work should run on exactly one machine. A "run on exactly one of N hosts" mode needs a lease protocol between hosts that only expose SSH to each other; the alternative is explicit pinning, where the definition names every host it fires on.

## Decision

Each routine carries a `hosts:` list matched against the host-local `host_id`; there is no "any host" value in v1. Listing two hosts means two independent fires. Failover stays out of scope until a real routine needs it.

## Consequences

- Due computation stays purely host-local: no lease table, no network dependency, no split-brain modes to test.
- The semantics are trivially predictable from the YAML alone.
- Cost: no routine survives its pinned host being down, and adding leases later introduces a second, coordinated mode whose semantics diverge from everything shipped in v1.