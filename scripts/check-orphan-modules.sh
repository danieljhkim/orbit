#!/usr/bin/env bash
set -euo pipefail

# Every src/**/*.rs file (excluding mod.rs/lib.rs/main.rs and anything under a
# tests/ dir) must be reachable from the module tree: declared as `mod <stem>`
# in its directory's owning file (mod.rs, lib.rs, or main.rs), in the sibling
# file that owns the directory as a module (`foo.rs` next to `foo/`), or
# referenced by a `#[path = "...stem.rs"]` attribute anywhere in the crate.
# Otherwise the file is never compiled, linted, or tested — see
# docs/design/orbit-cleanup/orbitenginecleanup.md §1/§11.
#
# Under the sibling-test layout (docs/design-patterns/test_layout.md), every
# src/**/tests/<name>.rs file must likewise be declared as `mod <name>` in its
# own tests/mod.rs (or reached via a `#[path]` attribute). A file that looks
# like a registered test but isn't compiles to nothing and silently drops its
# coverage — see F2026-08-108.

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

if ! command -v rg >/dev/null 2>&1; then
  echo "check-orphan-modules: ripgrep (rg) is required; install it before running" >&2
  exit 1
fi

fail=0

is_declared() {
  local stem="$1"
  local dir="$2"
  local file="$3"

  local owner
  for owner in \
    "$dir/mod.rs" \
    "$dir/lib.rs" \
    "$dir/main.rs" \
    "$(dirname "$dir")/$(basename "$dir").rs"; do
    if [[ -f "$owner" ]] && rg -q "\\bmod[[:space:]]+(r#)?${stem}\\b[[:space:]]*[;{]" "$owner"; then
      return 0
    fi
  done

  local crate_src="${file%%/src/*}/src"
  local base
  base="$(basename "$file")"
  if rg -q "#\\[path[[:space:]]*=[[:space:]]*\"[^\"]*${base}\"" --glob '*.rs' "$crate_src" 2>/dev/null; then
    return 0
  fi

  return 1
}

while IFS= read -r -d '' file; do
  case "$file" in
    # Sibling unit-test trees (docs/design-patterns/test_layout.md).
    */tests/*) continue ;;
    # Cargo auto-discovers every file directly under src/bin/ as its own
    # binary crate root ([[bin]] path or implicit) — no `mod` needed.
    */src/bin/*) continue ;;
  esac

  dir="$(dirname "$file")"
  base="$(basename "$file")"
  stem="${base%.rs}"

  if ! is_declared "$stem" "$dir" "$file"; then
    echo "orphan module: ${file} — no mod ${stem}; declaration and no matching #[path] attribute"
    fail=1
  fi
done < <(find crates/*/src -type f -name '*.rs' \
  ! -name 'mod.rs' ! -name 'lib.rs' ! -name 'main.rs' -print0)

# Sibling tests/<name>.rs files must themselves be declared in their owning
# tests/mod.rs (or via #[path]) — see docs/design-patterns/test_layout.md.
while IFS= read -r -d '' file; do
  dir="$(dirname "$file")"
  base="$(basename "$file")"
  stem="${base%.rs}"

  if ! is_declared "$stem" "$dir" "$file"; then
    echo "orphan test module: ${file} — no mod ${stem}; declaration in ${dir}/mod.rs and no matching #[path] attribute"
    fail=1
  fi
done < <(find crates/*/src -type f -path '*/tests/*.rs' ! -name 'mod.rs' -print0)

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "orphan module guard passed"
