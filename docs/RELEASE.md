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
   - `publish-release` — uploads tarballs + `orbit-checksums.txt` to the
     GitHub Release.
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
