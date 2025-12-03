# RocketIndex

Rocket-fast F# language server and code indexer. Pure Rust, ~50MB RAM, no .NET required.

## Overview

RocketIndex provides codebase indexing and navigation for F# projects without the unbounded memory growth that plagues fsautocomplete. Written entirely in Rust using tree-sitter for parsing—no .NET runtime required.

### Features

- **Bounded, predictable memory usage** (<50MB for large codebases)
- **Fast codebase indexing and navigation**
- **SQLite-backed index** (persistent symbol store with optional type extraction data)
- **Go-to-definition** across files
- **Dependency graph traversal** ("spider" from entry point)
- **Clean CLI** for AI agent tooling
- **Zed extension** ("F# Fast") with syntax highlighting

### Non-Goals

- Feature parity with fsautocomplete
- Type-aware completions or hover information
- Refactoring tools (AI agents handle this)
- MCP integration (causes context bloat)

### Known Limitations

This tool uses syntactic analysis only (no .NET runtime). Key limitations:

- **No type awareness** — Cannot resolve overloaded methods or infer types
- **No external dependencies** — NuGet packages and FSharp.Core are not indexed
- **Approximate resolution** — Spider dependency graphs are best-effort

## Project Structure

```
rocket-index/
├── crates/
│   ├── rocketindex/        # Core library for symbol extraction and indexing
│   ├── rocketindex-cli/    # Command-line tool
│   └── rocketindex-lsp/    # Language server protocol implementation
└── extensions/
    └── zed-fsharp/         # Zed editor extension (F# Fast)
```

## Installation

### From Source

```bash
cargo build --release
```

The binaries will be available at:
- `target/release/rocketindex`
- `target/release/rocketindex-lsp`

## CLI Usage

All commands output JSON by default for easy integration with AI agents.

```bash
# Build/rebuild the index for current directory
$ rocketindex build

# Find definition (JSON output)
$ rocketindex def "PaymentService.processPayment"
# {"file": "src/Services.fs", "line": 42, "column": 5, ...}

# Human-readable output
$ rocketindex def "PaymentService.processPayment" --format pretty

# Spider from entry point
$ rocketindex spider "Program.main" --depth 5

# List all symbols matching pattern
$ rocketindex symbols "Payment*"

# Suppress progress output (useful in scripts)
$ rocketindex build --quiet
```

## AI Agent Integration

RocketIndex is designed to be the "SQLite of code indexing" for AI agents. It provides a CLI-first, JSON-native interface that works with any agent (Claude Code, Aider, Cursor, etc.).

### CLI Contract

- **Output**: JSON by default (`--format json`). Use `--format pretty` for human-readable output.
- **Errors**: Printed to stderr in JSON format when using JSON output.
- **Exit Codes**:
    - `0`: Success
    - `1`: Not found (valid query, no results)
    - `2`: Error (invalid input, missing file, etc.)

### Claude Code Integration

Drop-in slash commands are available in `integrations/claude-code/`. Copy these to your `.claude/commands/` directory to enable:
- `/rocketindex-def`: Find symbol definition
- `/rocketindex-spider`: Spider dependency graph

### Claude Skills

A template for Claude Skills is available in `integrations/claude-skill/SKILL.md`.

## SQLite Storage & Type Extraction

The `rocketindex build` command writes every symbol, reference, and `open` directive into a SQLite database at `.rocketindex/index.db`. This persistent store is used by both the CLI and the LSP so large workspaces can be loaded without rebuilding an in-memory index each run.

Optional semantic type data can be added by running the `scripts/extract-types.fsx` helper (automatically triggered by `rocketindex build --extract-types`). The script uses FSharp.Compiler.Service to record function signatures and type members, which are merged into the same SQLite file for richer hover output and member-resolution heuristics.

## Development

### Prerequisites

- Rust 1.75+
- Cargo

### Building

```bash
cargo build
```

### Testing

```bash
cargo test --all
```

## License

MIT - Copyright (c) 2025 Dawson Design Ltd.
