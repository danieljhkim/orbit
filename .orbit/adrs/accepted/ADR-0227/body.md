## Context
Hostname-derived strings and per-workspace transport targets can silently redirect a machine or elevate repository configuration.
## Decision
Assign every machine an immutable generated machine_id, keep the registry at the hub, and pin the one hub machine_id out of band in machine-local mcp.toml.
## Consequences
- Names resolve once at binding time and persisted records retain the stable identity.
- Cost: bootstrap transfers the hub identity out of band and registry/trust drift requires explicit diagnosis.