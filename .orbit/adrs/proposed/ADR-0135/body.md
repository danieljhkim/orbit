## Context
Task sync needs to fetch and push to a git remote on every mutation. The team already has an authenticated relationship with that remote (SSH keys, HTTPS tokens, SSO-wrapped credentials, SSH agent, etc.). Building a separate auth surface for Orbit would duplicate that machinery and create a separate credential-rotation problem.

## Decision
Task sync uses the system git credential helper for fetch/push. There is no Orbit-specific token, no separate ACL, no separate authentication. If the operator can `git push origin main`, they can `orbit task add` against the registry on the same remote. Failures (expired tokens, revoked SSH keys) surface as the same errors `git` itself would produce.

## Consequences
- No new auth surface to defend.
- Registry access is bounded by the same ACL that bounds code access.
- Cost: short-lived auth tokens (e.g., SSO-wrapped 8-hour tokens) cause `task add` to fail mid-day at refresh time. Orbit cannot mitigate this without owning auth, which it deliberately does not.

---
