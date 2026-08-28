#!/usr/bin/env bash
# Exercise the repository-owned Codex plugin validator at its supported
# Python interpreter boundary.
set -euo pipefail

repo_root="${1:-${ORBIT_REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}}"
validator="$repo_root/scripts/validate-codex-plugin.sh"

if ! command -v python3 >/dev/null 2>&1; then
  echo "test-validate-codex-plugin: required binary 'python3' not on PATH" >&2
  exit 2
fi

if ! grep -Fqx 'from __future__ import annotations' "$validator"; then
  echo "test-validate-codex-plugin: validator must defer annotations for Python 3.9" >&2
  exit 1
fi

python3 - <<'PY'
from __future__ import annotations

from typing import Any


def accepts_optional_mapping(value: dict[str, Any] | None) -> list[str]:
    return [] if value is None else list(value)


assert accepts_optional_mapping(None) == []
assert accepts_optional_mapping({"supported": True}) == ["supported"]
PY

fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/orbit-codex-plugin.XXXXXX")"
trap 'rm -rf -- "$fixture_root"' EXIT

"$repo_root/scripts/sync-plugin-skills.sh" --check >/dev/null
cp -R "$repo_root/plugin" "$fixture_root/plugin"
mkdir -p "$fixture_root/.agents/plugins"
cp "$repo_root/.agents/plugins/marketplace.json" "$fixture_root/.agents/plugins/marketplace.json"

"$validator" "$fixture_root"
