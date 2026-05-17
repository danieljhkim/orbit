.PHONY: help build release run check test fmt fmt-check clippy clean install uninstall dev watch audit tree ci ci-fast bench check-design-docs stability release-check

# ------------------------------------------------------------
# Config
# ------------------------------------------------------------
CARGO ?= cargo
BINARY := orbit
BIN_CRATE := orbit-cli
# Crate sources live under orbit/ (see root Cargo.toml workspace members).
BIN_CRATE_PATH := crates/$(BIN_CRATE)
WORKSPACE := --workspace
INSTALL_PROFILE ?= release
INSTALL_BIN_DIR ?= $(HOME)/.cargo/bin

# Detect profile
PROFILE ?= debug
ifeq ($(PROFILE),release)
	CARGO_PROFILE := --release
	TARGET_DIR := target/release
else
	CARGO_PROFILE :=
	TARGET_DIR := target/debug
endif

ifeq ($(INSTALL_PROFILE),release)
	INSTALL_CARGO_PROFILE := --release
	INSTALL_TARGET_DIR := target/release
else
	INSTALL_CARGO_PROFILE :=
	INSTALL_TARGET_DIR := target/debug
endif

# ------------------------------------------------------------
# Help
# ------------------------------------------------------------
help:
	@echo "Orbit Workspace Make Targets"
	@echo ""
	@echo "  make build        Build workspace (PROFILE=release optional)"
	@echo "  make release      Build optimized release binary"
	@echo "  make run ARGS=... Run CLI binary"
	@echo "  make dev ARGS=... Run debug binary directly"
	@echo "  make check        Type-check entire workspace"
	@echo "  make test         Run all tests"
	@echo "  make fmt          Format code"
	@echo "  make fmt-check    Check formatting"
	@echo "  make clippy       Lint with clippy (deny warnings)"
	@echo "  make bench        Run graph build benchmark (ARGS=... optional)"
	@echo "  make audit        Cargo audit (security)"
	@echo "  make tree         Print dependency tree"
	@echo "  make ci           Full CI pass (clippy + tests + doc + guardrails; also runs on PRs)"
	@echo "  make ci-fast      Pre-handoff gate for agents (fmt-check + guardrail scripts; no compile)"
	@echo "  make check-design-docs  Flag docs/design/* stale relative to referenced code"
	@echo "  make stability    Verify per-crate stability tier markers"
	@echo "  make release-check  Verify /plugin install orbit version lockstep (see docs/RELEASE.md)"
	@echo "  make install      Install CLI locally (INSTALL_PROFILE=debug optional)"
	@echo "  make uninstall    Remove installed binary"
	@echo "  make clean        Clean build artifacts"
	@echo "  make watch        Continuous check + test"

# ------------------------------------------------------------
# Build
# ------------------------------------------------------------
build:
	$(CARGO) build $(WORKSPACE) $(CARGO_PROFILE)

release:
	$(CARGO) build -p $(BIN_CRATE) --bin $(BINARY) --release

# ------------------------------------------------------------
# Run
# ------------------------------------------------------------
run:
	$(CARGO) run -p $(BIN_CRATE) --bin $(BINARY) -- $(ARGS)

# Direct execution (after build)
dev: build
	$(TARGET_DIR)/$(BINARY) $(ARGS)

# ------------------------------------------------------------
# Quality
# ------------------------------------------------------------
check:
	$(CARGO) check $(WORKSPACE)

test:
	$(CARGO) test $(WORKSPACE)

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

clippy:
	$(CARGO) clippy $(WORKSPACE) --all-targets -- -D warnings

bench:
	$(CARGO) run -p orbit-knowledge --example graph_build --release -- $(ARGS)

# Security audit (requires cargo-audit)
audit:
	$(CARGO) audit || echo "Install cargo-audit via: cargo install cargo-audit"

# Dependency tree inspection
tree:
	$(CARGO) tree -e features

# Full CI pass
ci:
	./scripts/ci-guardrails.sh

# Pre-handoff gate for agents: fast checks, no compile. Full make ci runs on PRs.
ci-fast:
	cargo fmt --all -- --check
	./scripts/check-dependency-direction.sh
	./scripts/check-cli-imports.sh
	./scripts/check-stability.sh
	./scripts/check-learning-layout.sh

# Flag design docs older than the code they reference
check-design-docs:
	$(CARGO) run --quiet -p $(BIN_CRATE) --bin $(BINARY) -- design check

# Verify every workspace crate declares its stability tier
stability:
	./scripts/check-stability.sh

# Verify /plugin install orbit version invariant before cutting a release
release-check:
	./scripts/release-check.sh

# ------------------------------------------------------------
# Install
# ------------------------------------------------------------
install:
	$(CARGO) build -p $(BIN_CRATE) $(INSTALL_CARGO_PROFILE)
	install -d $(INSTALL_BIN_DIR)
	install -m 755 $(INSTALL_TARGET_DIR)/$(BINARY) $(INSTALL_BIN_DIR)/$(BINARY)

uninstall:
	rm -f $(INSTALL_BIN_DIR)/$(BINARY)

# ------------------------------------------------------------
# Clean
# ------------------------------------------------------------
clean:
	$(CARGO) clean

# ------------------------------------------------------------
# Dev Loop
# ------------------------------------------------------------
watch:
	$(CARGO) watch -x "check" -x "test"
