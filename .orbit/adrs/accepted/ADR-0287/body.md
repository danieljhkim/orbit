## Context
The versioned SQLite ledger skips migrations already recorded, so editing the v1 baseline alone changes fresh databases but not legacy databases. A long-lived worker can also retain an older runtime while another Orbit process advances the shared database, causing a late downgrade-guard error only when telemetry reopens the store after agent work. The alternatives were to rerun the mutable baseline on every open or to preserve shipped versions and require append-only upgrades plus an early worker compatibility check.

## Decision
Treat the v1 baseline as an immutable historical artifact guarded by a structural fingerprint. Every schema change after v1 must use a new append-only migration, and tests compare a fresh database with a v1 database upgraded through every registered version. Pipeline workers reopen the store once before claiming the run so compatible pending migrations apply and incompatible newer schemas fail before agent work. Invocation telemetry remains non-fatal and records durable degradation evidence.

## Consequences
- Fresh and legacy databases converge through the same ordered registry, and baseline-only edits fail structural tests.
- Long-lived workers pay one additional SQLite open before claiming a run and cannot discover schema incompatibility only after useful agent work.
- Cost: schema authors must append a migration and update structural expectations instead of editing the fresh baseline in place; the baseline fingerprint is intentionally strict.