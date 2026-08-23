---
type: runbook
summary: Cut and verify an Orbit release across Cargo, GitHub artifacts, Homebrew, and npm.
tags: [operations, release, npm, signing]
paths: [".github/workflows/release.yml", ".github/workflows/smoke-npm-install.yml", "npm/**", "scripts/release-check.sh"]
related_features: [orbit-docs-plugin]
---

# Release Orbit

This runbook keeps the Cargo workspace version, the `@orbit-tools/cli` npm
proxy, and the GitHub Release tag in lockstep. The npm package downloads the
native binary for its own version during postinstall, so version drift either
installs the wrong release or fails the download.

See [RELEASING.md](../../RELEASING.md) for the higher-level release checklist,
branch promotion policy, and CHANGELOG process.

## Account setup (one-time)

The `@orbit-tools` scope has publish-time 2FA enabled, so npm releases are
published manually from a maintainer's machine and prompt for an OTP. No
`NPM_TOKEN` secret is needed in this repository.

GitHub Releases require `ORBIT_RELEASE_SIGNING_KEY_PEM`, a PEM-encoded private
key whose public half matches
[`npm/release-signing.pub`](../../npm/release-signing.pub). The release
workflow signs `orbit-checksums.txt` as `orbit-checksums.txt.sig`;
`install.sh`, the npm postinstall, and `orbit semantic install` authenticate
that signature before trusting release-hosted SHA-256 values.

The installers carry a small release-signing trust set:

- `orbit-release-key-3` is the current signing path, valid through
  `2029-12-31` and not revoked.
- `orbit-release-key-4` is the pre-staged successor, valid through
  `2030-12-31` and not revoked. Its PEM is a placeholder; replace it with a
  real independently held keypair before rotation.

Key IDs are generation labels, not dates. During verification the installers
try each known public key, then reject a matching key when its `not_after`
date has passed or its `revoked_at` field is set.

> **Operator custody requirement.** Keep the successor private key in custody
> independent from the primary. Storing both private halves together defeats
> the overlap window's protection.

## Steps to cut a release

1. **Bump Cargo and npm together.**

   - Update `[workspace.package].version` in
     [`Cargo.toml`](../../Cargo.toml).
   - Update `.version` in
     [`npm/package.json`](../../npm/package.json). The npm postinstall in
     [`npm/scripts/install-binary.js`](../../npm/scripts/install-binary.js)
     derives the GitHub binary tag from this version.
   - Run `cargo update --workspace` to refresh `Cargo.lock` without
     third-party dependency drift.

   For a major version bump, first follow
   [RELEASING.md's CHANGELOG archiving policy](../../RELEASING.md#changelog-archiving).

2. **Run `make release-check`.** Before the new npm package and GitHub Release
   exist, this normally reports only local-to-remote drift against the previous
   version. Any Cargo-to-`npm/package.json` mismatch is a local error and must
   be fixed before tagging.

3. **Keep the npm smoke current.** If this release changes any non-interactive
   command driven by
   [`scripts/smoke-npm-install.sh`](../../scripts/smoke-npm-install.sh)
   (`orbit init`, `workspace init`, or `mcp serve`), land the script
   update on the same commit as the tag.

4. **Commit and merge the release preparation to `agent-main`.** Keep the
   Cargo and npm version bumps, lockfile refresh, and any smoke update together.

5. **Tag the merge commit.**

   ```bash
   git tag -a vX.Y.Z -m "orbit vX.Y.Z"
   git push origin vX.Y.Z
   ```

6. **Watch [`.github/workflows/release.yml`](../../.github/workflows/release.yml).**
   Its jobs:

   - build four platform CLI tarballs and the supported semantic companions;
   - generate and sign the combined checksum manifest, then create the GitHub
     Release;
   - update the Homebrew tap;
   - smoke the tagged shell installer on macOS and Ubuntu, including semantic
     companion installation where supported.

   Require every job to pass. Then follow
   [RELEASING.md §10b](../../RELEASING.md#10b-promote-to-main) to promote
   `agent-main` to `main`, and
   [§10c](../../RELEASING.md#10c-post-merge-back-merge-to-agent-main) to
   back-merge in the same session.

7. **Publish npm manually** from the merged commit:

   ```bash
   cd npm
   npm publish --access public
   # Enter the OTP from your authenticator when prompted.
   ```

   Manual publication cannot use GitHub OIDC provenance. Keep the interval
   between the GitHub Release and npm publish short; until npm updates,
   `npx -y @orbit-tools/cli@latest` still selects the previous release.

8. **Verify after npm publish.**

   ```bash
   make release-check
   ./scripts/smoke-npm-install.sh
   ```

   Re-run
   [`.github/workflows/smoke-npm-install.yml`](../../.github/workflows/smoke-npm-install.yml)
   with the release tag in its `tag` input. The tag-triggered run often starts
   before npm publish and is expected to fail its version assertion; the
   post-publish versioned run must be green.

## Continuous npm-install verification

The `smoke-npm-install.yml` workflow runs on macOS and Ubuntu weekly, on every
`v*` tag, and by manual dispatch. It executes
`npx -y @orbit-tools/cli@latest --version`, thereby exercising the npm
postinstall binary download and checksum/signature verification. It then
initializes an isolated Orbit home and workspace and drives `orbit mcp serve`
through JSON-RPC `initialize` and `tools/list`.

The smoke runs against published artifacts, not the local package. Its pass
criterion is that the installed version matches the requested tag when one is
present and that the MCP response advertises Orbit tools. Windows is not
covered; the npm proxy publishes only macOS and Linux packages.

The tag assertion can be tested without network access:

```bash
./scripts/smoke-npm-install.sh --dry-run-version-assertion
```

## Npm-install smoke: two artifacts

The workflow checks out `scripts/smoke-npm-install.sh` from the trigger ref,
while the script installs `@orbit-tools/cli@latest` from npm. Those are
independently versioned until npm publication completes.

| Trigger | Script comes from | CLI comes from |
|---|---|---|
| push of tag `vX.Y.Z` | that tag | npm `@latest` at run time |
| `workflow_dispatch` | the selected ref | npm `@latest` at run time |
| weekly cron | the default branch | npm `@latest` at run time |

A green versioned smoke needs both a published `@latest` matching the tag and
a script on the selected ref that speaks the CLI's current non-interactive
contract.

If the post-publish smoke is red:

1. Confirm `npm view @orbit-tools/cli version` equals the tag without `v`.
2. Read the failing step and the captured stdout/stderr.
3. Classify the failure:
   - A script-only contract issue is fixed on `agent-main`; dispatch that ref.
     Do not retag or publish a second npm version.
   - A bad published binary, package, or release asset requires the next patch
     release. Tags and npm versions are immutable.
4. A maintainer with Actions write permission can dispatch:

   ```bash
   gh workflow run smoke-npm-install.yml --ref agent-main
   gh workflow run smoke-npm-install.yml --ref vX.Y.Z -f tag=vX.Y.Z
   ```

Do not dispatch an old tag after a script-only fix; it checks out the old
script again.

## Release signing key rotation and revocation

Normal rotation uses an overlap window:

1. Generate the successor keypair offline. Add the public half to the trust
   sets in `install.sh` and
   [`npm/scripts/install-binary.js`](../../npm/scripts/install-binary.js)
   with a new key ID and `not_after` date.
2. Publish a release and npm package that still sign with the old key while
   both installers trust old and new keys.
3. Update `ORBIT_RELEASE_SIGNING_KEY_PEM` and
   [`npm/release-signing.pub`](../../npm/release-signing.pub), then cut the
   first release signed by the successor.
4. After the overlap window, remove the old key or mark its `revoked_at`
   date.

Emergency revocation only protects users who upgrade: already-published npm
packages retain their embedded trust sets. Publish a patch signed by a
non-revoked key, deprecate affected npm versions, and announce the required
upgrade.

Installer trust overrides are explicit trust-boundary changes.
`ORBIT_RELEASE_TRUSTED_KEYS_FILE` replaces the full key set and requires
`ORBIT_RELEASE_TRUSTED_KEYS_FILE_ACKNOWLEDGE_TRUST_CHANGE=1`.
`ORBIT_RELEASE_PUBLIC_KEY_FILE` is a deprecated single-key override and
requires its matching acknowledgement variable. The two overrides cannot be
used together.

## What `make release-check` enforces

[`scripts/release-check.sh`](../../scripts/release-check.sh) compares:

- `[workspace.package].version` in `Cargo.toml`;
- `.version` in `npm/package.json`;
- `npm view @orbit-tools/cli version`, when npm is available;
- the latest `gh release list -L 1` tag, when GitHub CLI access is available.

Missing `npm` or `gh` skips that remote source with a stderr note. Local
Cargo/npm drift always fails.

<!-- ORB-10995 -->
