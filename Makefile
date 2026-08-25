.PHONY: all check fmt shellcheck clippy test build build-wasm build-optimized

all: check

# Run all verification checks
check: fmt shellcheck clippy test

# Check Rust code formatting
fmt:
	cargo fmt --check

# Lint shell scripts (fails if shellcheck isn't installed, matching CI)
shellcheck:
	@command -v shellcheck >/dev/null 2>&1 || { echo "shellcheck not installed; install it before running make check."; exit 1; }
	shellcheck -x scripts/*.sh scripts/lib/*.sh

# Build Tholos WASM first (required by demo-consumer at compile time)
build-wasm:
	cargo build -p tholos --target wasm32v1-none --release

# Run Clippy with warnings treated as errors
clippy: build-wasm
	cargo clippy --workspace --all-targets -- -D warnings

# Run unit tests
test: build-wasm
	cargo test

# Build both workspace dependency WASM and the optimized deployable contract
build: build-wasm build-optimized

# Build the optimized, deployable contract WASM (requires Stellar CLI)
build-optimized:
	cd contracts/tholos && stellar contract build
