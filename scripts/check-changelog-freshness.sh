#!/usr/bin/env bash
# CHANGELOG freshness guardrail [ORB-10010].
#
# Fails when CHANGELOG.md carries no `## Unreleased` entries, so in-flight work
# can't silently skip its release note. Runs in `make ci-fast` (local) and
# `scripts/ci-guardrails.sh` (CI).
#
# Release exemption: RELEASING.md's release flow drafts the new `## <X.Y.Z>`
# section directly and drops the `Unreleased` header (see the v0.9.2 release
# commit), so an empty/missing Unreleased section is the *expected* state on a
# release commit and on every tree between a release and the next piece of
# logged work. The check therefore passes when either:
#   - RELEASE_PR=1 is set (explicit override for release automation), or
#   - HEAD's commit subject matches the documented release-commit pattern
#     (`chore: prepare vX.Y.Z release`, RELEASING.md step 8), or
#   - the most recent commit that touched CHANGELOG.md is such a release
#     commit — i.e. "the last release shipped and nothing new has been
#     logged yet", the normal post-release dev tree. (Skipped on shallow
#     clones, where file history is unavailable; CI's depth-1 checkout is
#     covered by the HEAD-subject rule on the release PR itself.)
set -euo pipefail

repo_root="${1:-${ORBIT_REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}}"
changelog="$repo_root/CHANGELOG.md"
release_pattern='^chore: prepare v[0-9]+\.[0-9]+\.[0-9]+ release'

if [[ ! -f "$changelog" ]]; then
  echo "error: CHANGELOG.md not found at $changelog" >&2
  exit 1
fi

# Extract the Unreleased section body (between `## Unreleased` and the next
# `## ` heading) and count content lines: anything that isn't blank and isn't
# a `###` sub-heading.
entry_count="$(awk '
  /^## / {
    if (in_section) exit
    if ($0 ~ /^## [Uu]nreleased[[:space:]]*$/) { in_section = 1; next }
  }
  in_section && !/^[[:space:]]*$/ && !/^###/ { count++ }
  END { print count + 0 }
' "$changelog")"

if [[ "$entry_count" -gt 0 ]]; then
  exit 0
fi

# --- Empty or missing Unreleased section: check the release exemptions. ---

if [[ "${RELEASE_PR:-0}" == "1" ]]; then
  echo "check-changelog-freshness: Unreleased is empty, but RELEASE_PR=1 is set; skipping." >&2
  exit 0
fi

if git -C "$repo_root" rev-parse --git-dir >/dev/null 2>&1; then
  head_subject="$(git -C "$repo_root" log -1 --format=%s 2>/dev/null || true)"
  if [[ "$head_subject" =~ $release_pattern ]]; then
    echo "check-changelog-freshness: HEAD is a release commit ('$head_subject'); empty Unreleased is expected." >&2
    exit 0
  fi

  if [[ "$(git -C "$repo_root" rev-parse --is-shallow-repository 2>/dev/null)" != "true" ]]; then
    last_changelog_subject="$(git -C "$repo_root" log -1 --format=%s -- CHANGELOG.md 2>/dev/null || true)"
    if [[ "$last_changelog_subject" =~ $release_pattern ]]; then
      echo "check-changelog-freshness: CHANGELOG.md last touched by release commit ('$last_changelog_subject'); no new work logged yet." >&2
      exit 0
    fi
  fi
fi

cat >&2 <<'EOF'
error: CHANGELOG.md has no entries under '## Unreleased'.

Every change that lands between releases should log a consumer-facing entry.
Add a bullet under the '## Unreleased' section (create the section beneath the
'# Changelog' title if it is missing).

If this tree is a release (Unreleased was just folded into a version section),
either commit with the documented release message ('chore: prepare vX.Y.Z
release', see RELEASING.md) or run with RELEASE_PR=1 to skip this check.
EOF
exit 1
