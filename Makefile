.PHONY: help build release run check test fmt clippy clean install uninstall dev watch audit tree ci

# ------------------------------------------------------------
# Config
# ------------------------------------------------------------
BINARY := orbit
BIN_CRATE := orbit-cli
# Crate sources live under orbit/ (see root Cargo.toml workspace members).
BIN_CRATE_PATH := orbit/$(BIN_CRATE)
WORKSPACE := --workspace

# Detect profile
PROFILE ?= debug
ifeq ($(PROFILE),release)
	CARGO_PROFILE := --release
	TARGET_DIR := target/release
else
	CARGO_PROFILE :=
	TARGET_DIR := target/debug
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
	@echo "  make clippy       Lint with clippy (deny warnings)"
	@echo "  make audit        Cargo audit (security)"
	@echo "  make tree         Print dependency tree"
	@echo "  make ci           Full CI pass"
	@echo "  make install      Install CLI locally"
	@echo "  make uninstall    Remove installed binary"
	@echo "  make clean        Clean build artifacts"
	@echo "  make watch        Continuous check + test"

# ------------------------------------------------------------
# Build
# ------------------------------------------------------------
build:
	cargo build $(WORKSPACE) $(CARGO_PROFILE)

release:
	cargo build -p $(BIN_CRATE) --release

# ------------------------------------------------------------
# Run
# ------------------------------------------------------------
run:
	cargo run -p $(BIN_CRATE) -- $(ARGS)

# Direct execution (after build)
dev: build
	$(TARGET_DIR)/$(BINARY) $(ARGS)

# ------------------------------------------------------------
# Quality
# ------------------------------------------------------------
check:
	cargo fmt --all
	cargo check $(WORKSPACE)

test:
	cargo test $(WORKSPACE)

fmt:
	cargo fmt --all

clippy:
	cargo clippy $(WORKSPACE) --all-targets --all-features -- -D warnings

# Security audit (requires cargo-audit)
audit:
	cargo audit || echo "Install cargo-audit via: cargo install cargo-audit"

# Dependency tree inspection
tree:
	cargo tree -e features

# Full CI pass
ci: fmt clippy test

# ------------------------------------------------------------
# Install
# ------------------------------------------------------------
install:
	cargo install --path $(BIN_CRATE_PATH) --force

uninstall:
	cargo uninstall $(BINARY) || true

# ------------------------------------------------------------
# Clean
# ------------------------------------------------------------
clean:
	cargo clean

# ------------------------------------------------------------
# Dev Loop
# ------------------------------------------------------------
watch:
	cargo watch -x "check" -x "test"