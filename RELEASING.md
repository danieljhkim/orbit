# Releasing Orbit

Runbook for cutting an Orbit release. Codified from [T20260510-23] (v0.4.0).

See also [docs/RELEASE.md](docs/RELEASE.md) for the npm package, plugin manifest, and GitHub Release publishing steps.

## Versioning policy

Pre-1.0 semver: `0.<minor>.<patch>`.

- **Breaking** → bump minor (e.g. `0.3.1` → `0.4.0`).
- **Non-breaking** → bump patch (e.g. `0.3.0` → `0.3.1`).

### What counts as breaking

- CLI command or flag removal/rename.
- MCP tool input or output schema change (including response shape — array → object counts).
- Activity/job YAML schema removal, rename, or load-time validation that rejects previously-parseable input.
- Task storage layout or task-field enum change requiring data migration.
- Seeded asset removal (skill, activity, job) that external agent prompts may reference.
- Workspace knowledge-graph schema version bump that invalidates cached selectors.

### What does NOT count as breaking

- Validation tightening that rejects inputs that were already invalid by spec.
- New guards that match documented behavior (e.g. MCP surface catching up to CLI).
- Internal module decomposition or refactors with no external API change.
- Performance changes.
- New optional fields with safe defaults.

When in doubt, ask the human during the breaking-change confirmation step (see below) — defaulting conservative, but don't auto-promote behavior tightening to breaking.

## Release checklist

### 1. Survey commits since last tag

```sh
git log v<prev>..HEAD --pretty='%h%x09%s' --no-merges
git log v<prev>..HEAD --pretty='%s' --no-merges | grep -oE 'T[0-9]{8}-[0-9]+' | sort -u
```

If the unique task ID count exceeds ~30, delegate the per-task lookups to a subagent. The subagent should call `orbit.task.show` for each ID, group findings by theme, and return a structured outline with breaking-change candidates flagged for human review.

Track unattributed commits (no task ID) separately — they still need to be reflected in the CHANGELOG.

### 2. Draft the CHANGELOG entry

Insert a new `## <X.Y.Z>` section at the top of `CHANGELOG.md`. Section order:

1. **Release scope** (optional) — 2–4 headline bullets. Use only when the release introduces a major subsystem, pivots positioning, or warrants a top-of-page narrative. Skip for routine patch releases.
2. **Breaking Changes** — only for minor bumps; one bullet per breaking item.
3. **Features**
4. **Fixes**
5. **Chores** — refactors, docs, release metadata.

Bullet shape:

```
- **Theme name**: one-sentence description that reads in isolation. ([T20260510-13], [T20260510-14])
```

Group related task IDs into a single themed bullet rather than emitting one bullet per task. Cite commit SHAs (`[commit abc1234]`) for items with no task ID.

### 3. Confirm breaking changes with the human

Surface the breaking-change candidate list before drafting the final section. Show each candidate with its task ID, title, and the reason it was flagged. Let the human accept, downgrade, or add to the list. Do not classify autonomously.

### 4. Bump versions

Four files change every release:

| File | Field |
|------|-------|
| `Cargo.toml` | `[workspace.package].version` |
| `Cargo.lock` | refresh via `cargo update --workspace` (no third-party drift) |
| `plugin/.claude-plugin/plugin.json` | `version` |
| `plugin/npm/package.json` | `version` |

The other `0.X.Y` matches in the repo (install-script doc comments, the website task pages, the Node engine pin in `website/package-lock.json`) are intentional — leave them.

### 5. Verify the build

```sh
make build
```

Must finish clean. `cargo update --workspace` should report only Orbit workspace members re-locked — investigate any third-party version movement before continuing.

### 6. Create the Orbit task

```
title:       Prepare v<X.Y.Z> release
type:        chore
tags:        ["release"]
context_files:
  - file:CHANGELOG.md
  - file:Cargo.toml
  - file:Cargo.lock
  - file:plugin/.claude-plugin/plugin.json
  - file:plugin/npm/package.json
```

Acceptance criteria: each of the four file bumps reports the new version, the CHANGELOG section is in place with the agreed structure, and every confirmed breaking change appears under Breaking Changes.

### 7. Human approval

Per `CLAUDE.md`: do not commit until the Orbit task is explicitly approved by the human. Approval transitions the task `proposed → backlog`; the implementing agent then `start`s it.

### 8. Commit

```sh
git -c user.name='<agent>' -c user.email='<agent-email>' commit \
  --author='<agent> <agent-email>' \
  -m "chore: prepare v<X.Y.Z> release [T<task-id>]

<one or two sentence description>"
```

Use the agent commit identity that matches the model running the release (`claude <noreply@anthropic.com>`, `codex <codex@orbit.local>`, etc.) — see existing `git log` for the canonical email per agent.

### 9. Tag

```sh
git tag -a v<X.Y.Z> -m "v<X.Y.Z>

See CHANGELOG.md for the full release notes. Highlights:
- ...
- ...
- N breaking changes (...)"
```

Annotated tag — never lightweight. Keep the message terse; CHANGELOG is the source of truth.

### 10. Push

```sh
git push origin <branch>
git push origin v<X.Y.Z>
```

Branch first, then tag — this lets release CI resolve the tag against an already-pushed commit.

### 10b. Promote to `main`

After the tag pushes and release CI goes green, open a PR `agent-main → main` so the release reaches the production branch:

```sh
gh pr create --base main --head agent-main \
  --title "release: v<X.Y.Z>" \
  --body "Promotes v<X.Y.Z>. See CHANGELOG.md."
```

Merge with a **merge commit** (not squash) so the release tag remains reachable from `main`'s history. The PR is a fast-forward unless a hotfix landed on `main` since the last release — in that case resolve via merge, not rebase. See [Hotfix flow](#hotfix-flow) for the inverse direction.

### 11. Mark the Orbit task done

Update with `status: done`, `implemented_by: <agent>`, and an `execution_summary` that records the commit SHA and tag. Future releases will discover this task via the `release` tag.

## Release CI

Pushing a `v*` tag triggers `.github/workflows/release.yml`:

- **`build-release`** — `cargo build -p orbit-cli --release --locked` against four targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`. Tarballs uploaded as workflow artifacts.
- **`publish-release`** — generates `orbit-checksums.txt` (SHA256) and creates the GitHub Release with the four tarballs + checksum file attached. Release notes are auto-generated by `softprops/action-gh-release`.
- **`bump-homebrew-tap`** — rewrites `Formula/orbit.rb` in the `danieljhkim/homebrew-tap` repo with the new version and the two macOS SHAs, then pushes via `secrets.TAP_GITHUB_TOKEN`. The formula is **macOS-only**; Linux users go through `install.sh`.
- **`smoke-install-macos`** / **`smoke-install-ubuntu`** — fetches `install.sh` from the tagged ref (`raw.githubusercontent.com/.../<tag>/install.sh`) and verifies `orbit --version`. Note: `install.sh` rides with the release commit — changes land in the same tag.

The npm publish step was removed from the tag workflow in v0.3.1; the npm proxy package is published manually if needed.

Watch the Actions tab after pushing the tag. Real failure modes seen historically:

- **`cargo build --locked` fails**: `Cargo.lock` was not refreshed after the version bump (step 4) — fix forward in the next patch.
- **Homebrew tap step**: `secrets.TAP_GITHUB_TOKEN` expired, or the tap repo branch protection rejected the push.
- **Smoke install**: a regression in `install.sh` itself, since the smoke test pulls it from the tagged ref. Verify locally before tagging if `install.sh` changed in this release.

## When something goes wrong

- **Tag pushed pointing at the wrong commit**: do NOT force-update the tag. Cut the next patch release with the fix instead.
- **Release CI fails after the tag landed**: leave the tag, fix forward in the next patch release. The GitHub Release can be re-run from the Actions UI once the underlying issue is resolved (if the failure was infrastructure, not artifact-correctness).
- **Breaking change discovered post-tag that wasn't in the CHANGELOG**: amend the next release's CHANGELOG with a backdated note rather than rewriting the prior section.

## Hotfix flow

For critical fixes against a released `main` (when waiting for the next `agent-main` release cycle isn't acceptable):

1. **Branch from `main`**:

   ```sh
   git checkout -b hotfix/<slug> main
   ```

2. **Land the fix via PR targeting `main`** (same CI gate as release PRs). Keep the diff minimal — hotfixes are not the place for refactors.

3. **Cut a patch release on `main`**: follow steps 1–10 of the [Release checklist](#release-checklist) but with `main` as the branch, ending with `git push origin main && git push origin v<X.Y.Z+1>`. Skip step 10b (promote) — the fix is already on `main`.

4. **Back-merge `main` → `agent-main`** in the same session — never defer:

   ```sh
   git checkout agent-main
   git merge --no-ff main
   git push origin agent-main  # or via PR if branch-protected
   ```

   This prevents the hotfix from being silently re-overwritten by the next `agent-main → main` release merge. The back-merge runs CI so regressions surface immediately.

5. If the hotfix touches a file with in-flight agent work on `agent-main`, resolve in the back-merge PR; do not rebase agent branches onto the new `agent-main` tip.
