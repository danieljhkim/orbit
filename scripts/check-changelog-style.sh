#!/usr/bin/env bash
# CHANGELOG style guardrail [ORB-10125].
#
# Fails when a bullet under `## Unreleased` is too long. Unreleased entries use
# the release bullet shape at write time — `- **Theme**: 1–2 sentences.
# ([ORB-XXXXX])` — so the changelog stays a terse consumer-facing note instead
# of ballooning into full PR descriptions. Detail (migration steps, rationale,
# rejected alternatives, test inventories) lives in the cited Orbit task / ADR /
# commit message; the task ID is the pointer. See RELEASING.md step 2.
#
# Scope: only the `## Unreleased` section is linted. Released `## <X.Y.Z>`
# sections are frozen history and are never touched. Runs in `make ci-fast`
# (local) and `scripts/ci-guardrails.sh` (CI).
#
# Caps (tunable via env, defaults chosen to give the release shape slack):
#   CHANGELOG_MAX_WORDS  words per top-level bullet (default 60)
#   CHANGELOG_MAX_LINES  physical (non-blank) lines per top-level bullet (default 3)
#
# A "bullet" is a top-level `- ` list item plus its continuation/sub-bullet
# lines, up to the next top-level bullet, `###` sub-heading, or blank line.
set -euo pipefail

repo_root="${1:-${ORBIT_REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}}"
changelog="$repo_root/CHANGELOG.md"
max_words="${CHANGELOG_MAX_WORDS:-60}"
max_lines="${CHANGELOG_MAX_LINES:-3}"

if [[ ! -f "$changelog" ]]; then
  echo "error: CHANGELOG.md not found at $changelog" >&2
  exit 1
fi

violations="$(awk -v max_words="$max_words" -v max_lines="$max_lines" '
  function flush() {
    if (entry_active && (words > max_words || lines > max_lines)) {
      printf "  L%d: %d words / %d lines  %s\n", start_line, words, lines, first
      found = 1
    }
    entry_active = 0
  }
  # Section boundaries: any `## ` heading closes the Unreleased section.
  /^## / {
    if (in_section) { flush(); in_section = 0 }
    if ($0 ~ /^## [Uu]nreleased[[:space:]]*$/) in_section = 1
    next
  }
  in_section {
    # `### Breaking Changes` / `### Highlights` sub-headings arent entries.
    if ($0 ~ /^###/) { flush(); next }
    # Blank line ends the current bullet.
    if ($0 ~ /^[[:space:]]*$/) { flush(); next }
    # A new top-level bullet starts an entry.
    if ($0 ~ /^- /) {
      flush()
      entry_active = 1
      start_line = NR
      words = NF
      lines = 1
      first = substr($0, 1, 72)
      next
    }
    # Continuation / sub-bullet line belongs to the current entry.
    if (entry_active) {
      words += NF
      lines += 1
    }
  }
  END { flush(); exit found }
' "$changelog")" && rc=0 || rc=$?

if [[ "$rc" -eq 0 ]]; then
  exit 0
fi

cat >&2 <<EOF
error: CHANGELOG.md has over-length bullet(s) under '## Unreleased'
(cap: ${max_words} words / ${max_lines} lines per bullet):

$violations

Unreleased entries use the release bullet shape at write time:

  - **Theme**: 1–2 sentences that read in isolation. ([ORB-XXXXX])

Move migration steps, rationale, rejected alternatives, and test inventories
into the cited Orbit task / ADR / commit message — the task ID is the pointer,
don't duplicate the detail here. Breaking changes get one extra line max, with
the migration as a phrase ('x removed -> use y'). See RELEASING.md step 2.

Only '## Unreleased' is linted; released sections are frozen history.
EOF
exit 1
