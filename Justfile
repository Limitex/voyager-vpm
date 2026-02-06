# List available tasks
default:
    @just --list

# Run the application (accepts arguments)
run *args:
    @cargo run -- {{args}}

# Build in release mode
build:
    @cargo build --release

# Run tests
test:
    @cargo test

# Auto-format and fix code
fmt:
    @cargo fmt --all
    @cargo clippy --fix --all-targets --all-features --allow-dirty --allow-staged -- -D warnings

# Check formatting and code validity
lint:
    @cargo fmt --all -- --check
    @cargo check --all-targets --all-features
    @cargo clippy --all-targets --all-features -- -D warnings

# Generate documentation
doc:
    @cargo doc --no-deps --all-features

# Remove build artifacts
clean:
    @cargo clean

# Pre-release checks (format -> lint -> test -> build)
release: fmt lint test build

# CI checks (lint -> test)
ci: lint test
