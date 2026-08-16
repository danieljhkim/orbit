---
type: runbook
summary: Cut and verify an Orbit release across plugin manifests, GitHub artifacts, Homebrew, and npm.
tags: [operations, release, plugins, npm, signing]
paths: [".github/workflows/release.yml", "plugin/**", "scripts/release-check.sh"]
related_features: [orbit-docs-plugin]
---

# Release Orbit

How to cut an Orbit release such that `/plugin install orbit` and
`codex plugin add orbit@orbit` work against the new version. The version
invariant is load-bearing: the npm package, the Claude and Codex plugin
manifests, and the GitHub Release tag must all agree, or the
`npx -y @orbit-tools/cli@latest mcp serve` indirection in
[`plugin/.mcp.json`](../../plugin/.mcp.json) and
[`plugin/.codex-plugin/plugin.json`](../../plugin/.codex-plugin/plugin.json)
downloads a binary that does not match the installed plugin manifest.

See also [RELEASING.md](../../RELEASING.md) for the higher-level release runbook and versioning policy.

## Account setup (one-time)

The `@orbit-tools` scope has **publish-time 2FA** enabled, and npm no longer
honors automation tokens to bypass it for this account. Releases publish to
npm **manually** from a maintainer's laptop, prompting for an OTP. No
`NPM_TOKEN` secret is needed in this repository.

GitHub Releases also require `ORBIT_RELEASE_SIGNING_KEY_PEM`, a PEM-encoded
private key whose public half matches
[`plugin/npm/release-signing.pub`](../../plugin/npm/release-signing.pub). The
release workflow signs `orbit-checksums.txt` as `orbit-checksums.txt.sig`;
`install.sh`, the npm postinstall, and `orbit semantic install` authenticate
that signature before trusting release-hosted SHA-256 values.

The installers carry a small release-signing trust set, not a single forever
key:

- `orbit-release-key-3` — current signing path, valid through `2029-12-31`,
  not revoked.
- `orbit-release-key-4` — pre-staged successor signing path, valid through
  `2030-12-31`, not revoked. **Placeholder PEM** — the matching private key
  is not yet held by release infrastructure. Replace before rotation.

Key IDs are stable generation labels (`key-3`, `key-4`, …). The numeric suffix
is a generation counter, not a date, so an ID survives the rotation that
promotes it from successor to primary without becoming confusing. Earlier
generations (`key-1`, `key-2`) were retired when the current npm package
dropped support for installing older binaries; older versions of the published
npm package retain those keys in their own embedded trust sets.

During verification the installers try each known public key, then reject a
matching key if its `not_after` date has passed or its `revoked_at` field is
set. A signature that matches none of the trusted keys is rejected as
untrusted.

> **Operator custody requirement.** Pre-staging the successor key only buys
> rotation speed if the successor private key is held in *independent* custody
> from the primary. If both private halves are stored together (same secrets
> manager, same machine), a single compromise gets both and the trust set's
> benefit collapses. Future generations must preserve this separation.

## Steps to cut a release

Each step names the exact file or command. Do them in order.

1. **Bump the npm package version** in
   [`plugin/npm/package.json`](../../plugin/npm/package.json) (`.version`).
   The npm postinstall in
   [`plugin/npm/scripts/install-binary.js`](../../plugin/npm/scripts/install-binary.js)
   derives the binary tag as `v${PKG.version}`; this field is the source of
   truth that gets in front of users.

   If this release is a **major** version bump, first work through the
   CHANGELOG archiving convention in
   [`RELEASING.md`](../../RELEASING.md#changelog-archiving) — it moves
   released history out of `CHANGELOG.md` before this runbook's steps touch
   any version fields. This runbook does not restate that policy.

2. **Bump the plugin manifest versions** in
   [`plugin/.claude-plugin/plugin.json`](../../plugin/.claude-plugin/plugin.json)
   and
   [`plugin/.codex-plugin/plugin.json`](../../plugin/.codex-plugin/plugin.json)
   (`.version`). Both must match step 1.

3. **Run `make release-check`.** Pre-tag, it will exit non-zero because
   `npm view @orbit-tools/cli version` and the latest `gh release list -L 1`
   tag still point at the previous version. **That is expected.** Read the
   stderr lines to confirm the only drift reported is `local > remote` on
   exactly the previous version — anything else means an unrelated regression
   in one of the files the check inspects.

4. **Commit the version bumps** and merge to `agent-main`, the development
   integration branch. One commit, one PR, one bump set — do not let the two
   plugin manifests or the npm package drift across commits. If this cycle
   changed any CLI the plugin-install smoke drives, land the
   [`scripts/smoke-plugin-install.sh`](../../scripts/smoke-plugin-install.sh)
   update in the same bump (or earlier). This is not the final release
   landing: after the tag and release CI, step 6 directs the required
   promotion to `main`.

5. **Push the matching tag** from the merge commit, after confirming the
   smoke script on that commit still speaks the new CLI (see
   [Plugin-install smoke](#plugin-install-smoke-two-artifacts)):

   ```bash
   git tag -a vX.Y.Z -m "orbit vX.Y.Z"
   git push origin vX.Y.Z
   ```

6. **Watch [`.github/workflows/release.yml`](../../.github/workflows/release.yml).**
   Five jobs gate the cut:

   - `build-release` — builds platform CLI binaries and standalone
     `orbit-search-companion-*` binaries.
   - `publish-release` — signs the combined `orbit-checksums.txt` and uploads
     CLI tarballs, companion binaries, `orbit-checksums.txt`, and
     `orbit-checksums.txt.sig` to the GitHub Release.
   - `bump-homebrew-tap` — updates the formula in `danieljhkim/homebrew-tap`.
   - `smoke-install-macos` — installs the tagged macOS arm64 CLI, then runs
     `orbit semantic install --json` with isolated runtime state.
   - `smoke-install-ubuntu` — installs the tagged Linux x86_64 CLI, then runs
     `orbit semantic install --json` with isolated runtime state inside an
     Ubuntu 24.04 container.

   The Linux CLI release binaries still build on Ubuntu 22.04 to keep the CLI
   runtime floor low. The semantic companion is stricter: ONNX Runtime requires
   glibc >= 2.38 at runtime, so Linux companion builds and semantic smoke tests
   use Ubuntu 24.04 or newer. Released semantic companion assets currently
   cover macOS arm64 and Linux x86_64/aarch64 with glibc >= 2.38. Intel macOS
   receives a CLI asset, but semantic search is unsupported because the
   companion has no x86_64-apple-darwin ONNX Runtime prebuilt.

   All five must be green before step 7. At that point, follow
   [`RELEASING.md` §10b](../../RELEASING.md#10b-promote-to-main) to promote
   `agent-main` to the production release branch (`main`), then follow
   [`RELEASING.md` §10c](../../RELEASING.md#10c-post-merge-back-merge-to-agent-main)
   to back-merge `main` to `agent-main` in the same session.

7. **Publish to npm manually.** From the merged commit on your laptop:

   ```bash
   cd plugin/npm
   npm publish --access public
   # Enter the OTP from your authenticator when prompted.
   ```

   `--provenance` requires GitHub OIDC and is not available for manual
   publishes from a laptop. Skip it.

   Brief window: between step 6 going green and this step completing,
   `bump-homebrew-tap` has already shipped the new formula but
   `npx @orbit-tools/cli@latest` still hands users the previous version.
   Keep this window short — publish to npm immediately after step 6.

8. **Verify.** After npm publish completes:

   - `make release-check` should now pass (all local manifests, npm, and the
     release tag agree).
   - Re-run
     [`.github/workflows/smoke-plugin-install.yml`](../../.github/workflows/smoke-plugin-install.yml)
     via `workflow_dispatch` **on the tag** (or on `agent-main` if the tagged
     script already matches this cycle's CLI). The on-tag push run usually
     fired before step 7 and is expected red; that first failure is not
     actionable. See [Plugin-install smoke](#plugin-install-smoke-two-artifacts).
   - Optionally re-run the smoke locally:

     ```bash
     ./scripts/smoke-plugin-install.sh
     ```

   Codex users who installed from the Git marketplace update by refreshing the
   marketplace snapshot and reinstalling the plugin:

   ```bash
   codex plugin marketplace upgrade orbit
   codex plugin add orbit@orbit
   ```

## Continuous verification

[`.github/workflows/smoke-plugin-install.yml`](../../.github/workflows/smoke-plugin-install.yml)
runs the smoke on `macos-15` and `ubuntu-22.04` weekly (Monday 12:00 UTC)
and on every `v*` tag. It pulls the published `@orbit-tools/cli@latest`
from npm, exercises the postinstall download + sha256 verification, and
drives the orbit MCP server through a JSON-RPC `initialize` + `tools/list`
handshake. It also installs the repository's Codex plugin into an isolated
`CODEX_HOME`, renders a fresh Codex task prompt to confirm the shared Orbit
skills are discovered, and calls the read-only `orbit.task.list` MCP tool
through the installed Codex MCP transport. The pass criterion is that the
response advertises Orbit MCP tools and the read-only call succeeds. (Tool
names are emitted with underscores on the wire — see
`crates/orbit-mcp/src/adapter/name_map.rs::sanitize_tool_name` — even though the
canonical selectors used in skills and CLI args are dot-form.)

The smoke runs against published artifacts, not the local working tree, so
it catches version drift that local builds would miss. The Codex plugin
installation uses the checked-out repository marketplace
([`.agents/plugins/marketplace.json`](../../.agents/plugins/marketplace.json))
so manifest and skill packaging regressions are caught before publication.
Windows is not covered — the npm proxy only ships `darwin` and `linux`
builds.

Installer environment overrides are trust-boundary changes:

- `ORBIT_INSTALL_REPO`, `ORBIT_VERSION`, and `ORBIT_INSTALL_BASE_URL` in
  `install.sh` change where release artifacts are selected from. They still
  require a valid checksum signature unless the caller also changes the
  trusted key. `ORBIT_INSTALL_BASE_URL` intentionally accepts any scheme
  supported by the downloader, including `file://` for tests and `http://` for
  controlled mirrors; signature verification preserves artifact integrity, but
  the URL transport is not a confidentiality boundary.
- The npm package always installs the binary tag matching its own
  `package.json` version (`v${PKG.version}`) — older binaries cannot be
  selected through this package. To pin a different release tag, use the
  shell installer with `ORBIT_VERSION`.
- `ORBIT_RELEASE_TRUSTED_KEYS_FILE` is the preferred override for
  deterministic installer tests and emergency operations: a full replacement
  trust set with key IDs, `not_after`, and `revoked_at` metadata. Each record
  is `key_id|not_after|revoked_at|public_key_path`; empty `not_after` means
  no expiry and empty `revoked_at` means active. It requires
  `ORBIT_RELEASE_TRUSTED_KEYS_FILE_ACKNOWLEDGE_TRUST_CHANGE=1`.
- `ORBIT_RELEASE_PUBLIC_KEY_FILE` is **deprecated** in favor of
  `ORBIT_RELEASE_TRUSTED_KEYS_FILE` — the trusted-keys manifest is a strict
  superset (it can express the single-key case as a one-row file *plus*
  expiry/revocation metadata). The old var is retained for back-compat and
  still requires `ORBIT_RELEASE_PUBLIC_KEY_FILE_ACKNOWLEDGE_TRUST_CHANGE=1`;
  installers log a deprecation notice when it's in use. Both overrides cannot
  be set simultaneously.

## Plugin-install smoke: two artifacts

The workflow checks out
[`scripts/smoke-plugin-install.sh`](../../scripts/smoke-plugin-install.sh)
from the **trigger ref**, then that script drives
`npx -y @orbit-tools/cli@latest` (npm) and the GitHub Release binary the
package pins. Those are independent versions.

| Trigger | Script comes from | CLI comes from |
|---|---|---|
| push of tag `vX.Y.Z` | that tag | npm `@latest` at run time |
| `workflow_dispatch` | the ref you selected | npm `@latest` at run time |
| weekly cron | the workflow file's default branch | npm `@latest` at run time |

A green smoke therefore needs **both** a published `@latest` that matches
the tag **and** a script on the chosen ref that speaks that CLI's current
non-interactive contract (`orbit init`, `workspace init`, `mcp serve`).

### Before the tag

If this cycle changed any command the smoke drives — most often
`orbit init --non-interactive` growing required flags — land the script
update on the **same commit as the tag** (or earlier on `agent-main`). A
follow-up commit cannot turn the on-tag run green, because the tag's
checkout is frozen.

Walk the script against a throwaway HOME before tagging:

```bash
# From a clean tree that contains the release commit.
# Uses npm @latest, so this is only decisive *after* step 7; before
# publish it still proves the script's flags match the binary you are
# about to ship if you point NPM_PKG at a locally packed tarball or
# run the same argv against the freshly-built `target/release/orbit`.
./scripts/smoke-plugin-install.sh
```

At minimum, grep the script for every `orbit` / `npx` invocation and
confirm each still works non-interactively against the new binary.

### After npm, the smoke is still red

1. Confirm `npm view @orbit-tools/cli version` equals the tag (no `v`).
   If it does not, step 7 did not finish — publish, then continue.
2. Read the failing step. The script now dumps both stdout and stderr for
   `orbit init` (`init.out` + `init.err`); the useful line is often on
   stdout, so an empty `--- stderr ---` block is not a silent failure.
3. Decide **script vs published package**:
   - **Script-only** (missing flags, HOME / `ORBIT_ROOT` sandbox, log
     capture): fix on `agent-main`. Dispatch against **that ref**, not
     the old tag. Do not retag. Do not publish a second npm version.
     v0.11.0 / [ORB-10849] is this case — host identity required
     `--host-name` and `--task-prefix` on fresh non-interactive init,
     and the tagged script did not pass them.
   - **Published CLI, npm package, or GitHub Release asset**: cut a
     patch (`vX.Y.Z+1`). Do **not** retag. npm publishes are immutable.
4. `gh workflow run smoke-plugin-install.yml --ref <ref>` needs Actions
   write. Agent tokens often get HTTP 403; a maintainer dispatches.

```bash
# After a script-only fix has landed on agent-main:
gh workflow run smoke-plugin-install.yml --ref agent-main

# After npm publish, when the tagged script is already correct:
gh workflow run smoke-plugin-install.yml --ref vX.Y.Z
```

Do not dispatch the old tag after a script fix — it will check out the
broken script again and fail the same way.

## Release signing key rotation and revocation

Normal rotation uses an overlap window:

1. Generate the successor keypair offline. Add the public half to the trust
   set in both `install.sh` and
   [`plugin/npm/scripts/install-binary.js`](../../plugin/npm/scripts/install-binary.js)
   with a new key ID and `not_after` date. Keep the old active key until at
   least one npm package containing both keys has been published.
2. Publish a release and npm package that still signs with the old key, but
   whose installers trust both old and new keys.
3. Update `ORBIT_RELEASE_SIGNING_KEY_PEM` and
   [`plugin/npm/release-signing.pub`](../../plugin/npm/release-signing.pub) to the
   successor key. Cut the next release signed by the successor key.
4. After the overlap window, remove the old key from the trusted set or set
   its `revoked_at` date if it should remain visible for audit history.

Emergency revocation is intentionally more disruptive:

> **⚠️ Emergency revocation only protects users who upgrade.** Already-published
> npm packages contain their old trust set permanently. Marking a key as
> `revoked_at` in the *current* trust set blocks **new** releases signed by the
> compromised key — it does **not** retroactively block users from running an
> already-published `@orbit-tools/cli@<old>` package, whose postinstall still
> carries the old trust set. `npm deprecate` and the release announcement are
> the only revocation mechanisms for those installs; even then, package
> managers that ignore deprecations will continue to execute the old
> postinstall. Plan the release announcement to push users to upgrade *before*
> the deprecation lands.

1. Mark the compromised key record with `revoked_at: YYYY-MM-DD` in both
   installers and publish a patch release signed by a non-revoked key.
2. Update `ORBIT_RELEASE_SIGNING_KEY_PEM` and `release-signing.pub` to the
   replacement key before cutting that patch release.
3. Deprecate every already-published npm version whose postinstall still
   trusts the compromised key, for example:

   ```bash
   npm deprecate '@orbit-tools/cli@<=X.Y.Z' \
     'Release signing key revoked; upgrade to a patched @orbit-tools/cli.'
   ```

Because npm publish is manual, the on-tag smoke run will fail if it fires
before step 7 completes. That is expected and not actionable on its own;
re-run via `workflow_dispatch` after publishing to npm, choosing the ref
as described in [Plugin-install smoke](#plugin-install-smoke-two-artifacts).
The weekly cron catches a lingering broken state.

## What `make release-check` enforces

The script at [`scripts/release-check.sh`](../../scripts/release-check.sh)
asserts equality across these sources, when each is reachable:

- `.version` in [`plugin/npm/package.json`](../../plugin/npm/package.json)
- `.version` in [`plugin/.claude-plugin/plugin.json`](../../plugin/.claude-plugin/plugin.json)
- `.version` in [`plugin/.codex-plugin/plugin.json`](../../plugin/.codex-plugin/plugin.json)
- `npm view @orbit-tools/cli version`
- `gh release list -L 1` (latest tag, leading `v` stripped)

It also runs
[`scripts/validate-codex-plugin.sh`](../../scripts/validate-codex-plugin.sh),
which checks the Codex manifest, repository marketplace entry, shared skill
paths, and the absence of user-specific absolute paths or
`CLAUDE_PROJECT_DIR` in Codex MCP configuration.

Missing `npm` or `gh` is treated as a skip with a stderr note, not a hard
failure, so the target stays usable on a fresh checkout without credentials.
Mismatch across any reachable sources exits non-zero — so the pre-tag failure
described in step 3 is by design.

## Out-of-band fixes

If a release lands and the plugin-install smoke fails, follow
[Plugin-install smoke](#plugin-install-smoke-two-artifacts) before
cutting a patch. The short form:

1. `npm view @orbit-tools/cli version` still on the previous release →
   finish the manual publish, then `workflow_dispatch` the **tag**.
2. npm matches the tag, failure is a missing smoke-script flag or
   sandbox → fix the script on `agent-main`, dispatch **that** ref, no
   retag, no second npm publish.
3. npm matches the tag and the published binary or package is wrong →
   patch release (`vX.Y.Z+1`). Do **not** retag — npm publishes are
   immutable and the marketplace already cached the broken assets.
