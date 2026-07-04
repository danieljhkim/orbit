# Security Policy

## Supported Versions

Orbit is pre-1.0 and ships from `main`. Security fixes land on `main` and the most recent tagged release; older tags do not receive backports.

| Version       | Supported          |
| ------------- | ------------------ |
| `main` (HEAD) | :white_check_mark: |
| Latest tag    | :white_check_mark: |
| Older tags    | :x:                |

## Reporting a Vulnerability

Please report security issues privately via GitHub: open the repository's **Security** tab and choose **Report a vulnerability** ([private vulnerability reporting](https://github.com/danieljhkim/orbit/security/advisories/new)).

Do **not** open a public issue, pull request, or discussion for suspected vulnerabilities.

Include enough detail to reproduce: affected version or commit, environment, steps, observed vs. expected behavior, and any proof-of-concept. A suggested fix is welcome but not required.

## What to Expect

This is a small project, so response is best-effort:

- **Acknowledgement:** within 7 days.
- **Triage and assessment:** within 30 days, including whether the report is accepted, declined, or needs more information.
- **Fix and disclosure:** coordinated with the reporter once a patch is available. Reporters are credited in the advisory unless they prefer to remain anonymous.

If a report is declined, you'll get a written explanation of why (out of scope, intended behavior, mitigated elsewhere, etc.).

## Scope

In scope:

- The `orbit` CLI, runtime, and crates published from this repository.
- Filesystem-scoping policy bypasses (`fsProfile`, `denyRead`, `denyModify`).
- Sandbox / process supervision escapes in `orbit-exec`.
- Audit log tampering or omission paths.
- Authentication, authorization, or origin-check bypasses on `orbit web serve` and `orbit mcp serve`.
- Credential handling and redaction for provider keys.

Out of scope:

- Vulnerabilities in upstream dependencies — please report those upstream. We'll bump the dependency once a fix is available.
- Issues that require an attacker who already has local code execution as the user running Orbit, or write access to the workspace, unless they cross a documented trust boundary.
- Social-engineering or phishing of project maintainers.
- Findings against forks or third-party redistributions of Orbit.

## Filesystem Policy Enforcement

Agent filesystem access is scoped by an `fsProfile` (`read` / `modify` globs)
plus global `denyRead` / `denyModify` rules, evaluated by `orbit-policy`.

**Enforcement layers differ by platform:**

| Platform | Layers |
| --- | --- |
| **macOS** | Policy evaluation **and** the `orbit-exec` seatbelt (`sandbox-exec`) profile — two independent layers. |
| **Linux** | Policy evaluation **only** — there is no OS-level sandbox. The policy check is the sole line of defense, so it must be correct on its own. |

**Symlink resolution.** Policy rules are matched against the *real*
filesystem location, not the requested path. Before matching, the requested
path is resolved with `Path::canonicalize` (following symlinks); for a
not-yet-existing target (a write/create) the nearest existing ancestor is
canonicalized and the remaining components are rejoined. A symlink inside an
allowed subtree that points into a denied subtree is therefore **denied**, and
a resolved path that escapes the workspace root is denied outright. This
resolution lives in `orbit-policy` (`PolicyEngine::check_resolved`) so the
guarantee holds regardless of the caller.

**Known limitation (TOCTOU).** There is an inherent time-of-check to
time-of-use gap between the policy decision and the actual filesystem
operation: an attacker who can race the filesystem (swap a resolved path for a
symlink between check and use) is an OS-level concern outside what the policy
layer can close. On macOS the seatbelt provides a second enforcement point; on
Linux, treat a workspace an attacker can concurrently mutate as untrusted.
