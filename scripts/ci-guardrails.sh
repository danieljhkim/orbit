#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
# Keep examples covered by clippy's all-targets pass, but avoid re-running
# their empty test harnesses in the test phase.
if cargo nextest --version >/dev/null 2>&1; then
  cargo nextest run --workspace --lib --bins --tests
else
  echo "cargo-nextest not found; falling back to cargo test" >&2
  cargo test --workspace --lib --bins --tests
fi
cargo test --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
"$repo_root/scripts/check-dependency-direction.sh"
"$repo_root/scripts/check-cli-imports.sh"
"$repo_root/scripts/check-stability.sh"
"$repo_root/scripts/check-learning-layout.sh"
"$repo_root/scripts/check-artifact-redaction-boundary.sh"
