#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

# Several guardrail scripts shell out to ripgrep; a missing `rg` degrades
# them silently (empty search results read as "clean" or, worse, as false
# violations). Fail fast instead. [ORB-10021]
if ! command -v rg >/dev/null 2>&1; then
  echo "ci-guardrails: ripgrep (rg) is required; install it before running" >&2
  exit 1
fi

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
# The replay smokes require an explicit feature, so the default workspace
# clippy pass skips them. Keep that opt-in path compiled and linted too.
# orbit-core rides along: its replay-backed v2_host test needs the same
# transport, and `orbit-core/replay` forwards to `orbit-engine/replay`, so one
# pass covers both crates' opt-in configuration. [ORB-10414] [ORB-10434]
cargo clippy -p orbit-engine -p orbit-core --all-targets --features orbit-core/replay -- -D warnings
# The converted v2 integration tests are included in the default test surface;
# exercise the replay-gated cases separately so both configurations run.
if cargo nextest --version >/dev/null 2>&1; then
  cargo nextest run --workspace --lib --bins --tests
  cargo nextest run -p orbit-engine -p orbit-core --features orbit-core/replay --lib --bins --tests
else
  echo "cargo-nextest not found; falling back to cargo test" >&2
  cargo test --workspace --lib --bins --tests
  cargo test -p orbit-engine -p orbit-core --features orbit-core/replay --lib --bins --tests
fi
cargo test --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
# Supply-chain gate: dependency advisories + license allow-list (deny.toml).
# [ORB-00416] Soft-presence like nextest above — CI installs a pinned version;
# local runs without the tool warn instead of hard-failing.
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check
else
  echo "cargo-deny not found; skipping supply-chain gate (install: cargo install cargo-deny --locked)" >&2
fi
"$repo_root/scripts/generate-doc-indexes.sh" --check
"$repo_root/scripts/check-installer-pubkey.sh"
"$repo_root/scripts/test-installer-security.sh"
"$repo_root/scripts/check-dependency-direction.sh"
"$repo_root/scripts/check-cli-imports.sh"
"$repo_root/scripts/check-terminal-state-guard.sh"
"$repo_root/scripts/check-history-note-size.sh"
"$repo_root/scripts/check-stability.sh"
"$repo_root/scripts/check-learning-layout.sh"
"$repo_root/scripts/check-artifact-redaction-guardrail.sh"
"$repo_root/scripts/check-changelog-style.sh"
"$repo_root/scripts/check-error-translation.sh"
"$repo_root/scripts/check-orphan-modules.sh"
