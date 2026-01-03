# RocketIndex task runner
# Run `just` or `just --list` to see available commands

# Default: show available recipes
default:
    @just --list

# === Development ===

# Run all tests
test:
    cargo test --all

# Run tests with output visible
test-verbose:
    cargo test --all -- --nocapture

# Check formatting and run lints
check:
    cargo fmt --all -- --check
    cargo clippy --all -- -D warnings

# Format code
fmt:
    cargo fmt --all

# Build release binary
build:
    cargo build --release

# Build debug binary
build-debug:
    cargo build

# Run clippy with warnings as errors
lint:
    cargo clippy --all -- -D warnings

# === Testing ===

# Setup test fixtures (minimal, ci, or full)
fixtures mode='minimal':
    ./scripts/setup-test-fixtures.sh {{mode}}

# Run fixture tests against minimal fixtures
test-fixtures: build
    ./scripts/test-fixtures.sh

# Run tests against real-world repositories
test-repos *args='':
    ./scripts/test-real-repos.sh {{args}}

# Generate markdown report from test results
test-report file:
    python3 ./scripts/generate-test-report.py {{file}}

# === Benchmarks ===

# Run semantic benchmark (RocketIndex vs grep)
bench-semantic path='.':
    ./scripts/benchmarks/benchmark_semantic.sh {{path}}

# Run auto-refresh stress test
bench-stress:
    ./scripts/benchmarks/stress_test_auto_refresh.sh

# Run AI agent benchmark
bench-ai-agent *args='':
    ./scripts/benchmarks/benchmark_ai_agent.sh {{args}}

# Run formalized benchmark suite
bench-run *args='':
    ./scripts/benchmarks/run_benchmark.sh {{args}}

# === Release ===

# Bump version across all touchpoints
bump version:
    ./scripts/bump-version.sh {{version}}

# === Git Hooks ===

# Install git hooks (pre-commit, pre-push)
install-hooks:
    ./scripts/install-hooks.sh

# === CLI Shortcuts ===

# Build and run index on current directory
index: build
    ./target/release/rkt index

# Start watch mode (keeps index fresh during development)
watch: build
    ./target/release/rkt watch

# Start MCP server
serve: build
    ./target/release/rkt serve
