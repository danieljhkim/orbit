# Release Procedure

How to cut an Orbit release such that `/plugin install orbit` works against
the new version. The version invariant is load-bearing: the npm package, the
plugin manifest, and the GitHub Release tag must all agree, or the
`npx -y @orbit-tools/cli@latest mcp serve` indirection in
[`plugin/.mcp.json`](../plugin/.mcp.json) downloads a binary that does not
match the plugin manifest.

See also [../RELEASING.md](../RELEASING.md) for the higher-level release runbook and versioning policy.

## Account setup (one-time)

The `@orbit-tools` scope has **publish-time 2FA** enabled, and npm no longer
honors automation tokens to bypass it for this account. Releases publish to
npm **manually** from a maintainer's laptop, prompting for an OTP. No
`NPM_TOKEN` secret is needed in this repository.

GitHub Releases also require `ORBIT_RELEASE_SIGNING_KEY_PEM`, a PEM-encoded
private key whose public half matches
[`plugin/npm/release-signing.pub`](../plugin/npm/release-signing.pub). The
release workflow signs `orbit-checksums.txt` as `orbit-checksums.txt.sig`;
both `install.sh` and the npm postinstall authenticate that signature before
trusting release-hosted SHA-256 values.

The installers carry a small release-signing trust set, not a single forever
key:

- `orbit-release-key-1` — current signing path, valid through `2027-12-31`,
  not revoked.
- `orbit-release-key-2` — pre-staged successor signing path, valid through
  `2028-12-31`, not revoked.

Key IDs are stable generation labels (`key-1`, `key-2`, …). The numeric suffix
is a generation counter, not a date, so an ID survives the rotation that
promotes it from successor to primary without becoming confusing.

During verification the installers try each known public key, then reject a
matching key if its `not_after` date has passed or its `revoked_at` field is
set. A signature that matches none of the trusted keys is rejected as
untrusted.

> **Operator custody requirement.** Pre-staging the successor key only buys
> rotation speed if the successor private key is held in *independent* custody
> from the primary. If both private halves are stored together (same secrets
> manager, same machine), a single compromise gets both and the trust set's
> benefit collapses. The current `orbit-release-key-2` private half is held
> offline and is only loaded into `ORBIT_RELEASE_SIGNING_KEY_PEM` when the
> rotation runbook is executed. Future generations must preserve this
> separation.

## Steps to cut a release

Each step names the exact file or command. Do them in order.

1. **Bump the npm package version** in
   [`plugin/npm/package.json`](../plugin/npm/package.json) (`.version`).
   The npm postinstall in
   [`plugin/npm/scripts/install-binary.js`](../plugin/npm/scripts/install-binary.js)
   derives the binary tag as `v${PKG.version}`; this field is the source of
   truth that gets in front of users.

2. **Bump the plugin manifest version** in
   [`plugin/.claude-plugin/plugin.json`](../plugin/.claude-plugin/plugin.json)
   (`.version`). Must match step 1.

3. **Run `make release-check`.** Pre-tag, it will exit non-zero because
   `npm view @orbit-tools/cli version` and the latest `gh release list -L 1`
   tag still point at the previous version. **That is expected.** Read the
   stderr lines to confirm the only drift reported is `local > remote` on
   exactly the previous version — anything else means an unrelated regression
   in one of the files the check inspects.

4. **Commit the version bumps** and merge to the release branch
   (`agent-main`). One commit, one PR, one bump pair — do not let the two
   files drift across commits.

5. **Push the matching tag.** From the merge commit:

   ```bash
   git tag -a vX.Y.Z -m "orbit vX.Y.Z"
   git push origin vX.Y.Z
   ```

6. **Watch [`.github/workflows/release.yml`](../.github/workflows/release.yml).**
   Three jobs gate the cut:

   - `build-release` — builds platform binaries.
   - `publish-release` — signs `orbit-checksums.txt` and uploads tarballs,
     `orbit-checksums.txt`, and `orbit-checksums.txt.sig` to the GitHub
     Release.
   - `bump-homebrew-tap` — updates the formula in `danieljhkim/homebrew-tap`.

   All three must be green before step 7.

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

   - `make release-check` should now pass (all four sources agree).
   - The on-tag run of
     [`.github/workflows/smoke-plugin-install.yml`](../.github/workflows/smoke-plugin-install.yml)
     should be green on macOS and Linux. (If you re-run via
     `workflow_dispatch` it'll pull the freshly-published npm and exercise
     the full chain.)
   - Optionally re-run the smoke locally:

     ```bash
     ./scripts/smoke-plugin-install.sh
     ```

## Continuous verification

[`.github/workflows/smoke-plugin-install.yml`](../.github/workflows/smoke-plugin-install.yml)
runs the smoke on `macos-15` and `ubuntu-22.04` weekly (Monday 12:00 UTC)
and on every `v*` tag. It pulls the published `@orbit-tools/cli@latest`
from npm, exercises the postinstall download + sha256 verification, and
drives the orbit MCP server through a JSON-RPC `initialize` + `tools/list`
handshake. The pass criterion is that the response advertises at least one
`orbit_*` tool. (Tool names are emitted with underscores on the wire — see
`crates/orbit-mcp/src/adapter.rs::sanitize_tool_name` — even though the
canonical selectors used in skills and CLI args are dot-form.)

The smoke runs against published artifacts, not the local working tree, so
it catches version drift that local builds would miss. Windows is not
covered — the npm proxy only ships `darwin` and `linux` builds.

Installer environment overrides are trust-boundary changes:

- `ORBIT_INSTALL_REPO`, `ORBIT_VERSION`, and `ORBIT_INSTALL_BASE_URL` in
  `install.sh` change where release artifacts are selected from. They still
  require a valid checksum signature unless the caller also changes the
  trusted key. `ORBIT_INSTALL_BASE_URL` intentionally accepts any scheme
  supported by the downloader, including `file://` for tests and `http://` for
  controlled mirrors; signature verification preserves artifact integrity, but
  the URL transport is not a confidentiality boundary.
- `ORBIT_BINARY_VERSION` in the npm package changes the selected release tag
  while retaining signature verification.
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

## Release signing key rotation and revocation

Normal rotation uses an overlap window:

1. Generate the successor keypair offline. Add the public half to the trust
   set in both `install.sh` and
   [`plugin/npm/scripts/install-binary.js`](../plugin/npm/scripts/install-binary.js)
   with a new key ID and `not_after` date. Keep the old active key until at
   least one npm package containing both keys has been published.
2. Publish a release and npm package that still signs with the old key, but
   whose installers trust both old and new keys.
3. Update `ORBIT_RELEASE_SIGNING_KEY_PEM` and
   [`plugin/npm/release-signing.pub`](../plugin/npm/release-signing.pub) to the
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
re-run via `workflow_dispatch` after publishing to npm. The weekly cron
catches a lingering broken state.

## What `make release-check` enforces

The script at [`scripts/release-check.sh`](../scripts/release-check.sh)
asserts equality across four sources, when each is reachable:

- `.version` in [`plugin/npm/package.json`](../plugin/npm/package.json)
- `.version` in [`plugin/.claude-plugin/plugin.json`](../plugin/.claude-plugin/plugin.json)
- `npm view @orbit-tools/cli version`
- `gh release list -L 1` (latest tag, leading `v` stripped)

Missing `npm` or `gh` is treated as a skip with a stderr note, not a hard
failure, so the target stays usable on a fresh checkout without
credentials. Mismatch across any reachable sources exits non-zero — so
the pre-tag failure described in step 3 is by design.

## Out-of-band fixes

If a release lands and the smoke fails:

1. Re-run [`.github/workflows/smoke-plugin-install.yml`](../.github/workflows/smoke-plugin-install.yml)
   via `workflow_dispatch` to rule out a transient network failure or a
   "smoke fired before manual npm publish" race.
2. If the failure is reproducible, cut a patch release (`vX.Y.Z+1`) with
   the fix. Do **not** retag — npm publishes are immutable and the
   marketplace already cached the broken assets.
