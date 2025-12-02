# fsharp-tools

A minimal, lightweight F# language server optimized for AI-assisted coding workflows.

## Overview

fsharp-tools provides codebase indexing and navigation for F# projects without the unbounded memory growth that plagues fsautocomplete. Written entirely in Rust using tree-sitter for parsing—no .NET runtime required.

### Features

- **Bounded, predictable memory usage** (<50MB for large codebases)
- **Fast codebase indexing and navigation**
- **SQLite-backed index** (persistent symbol store with optional type extraction data)
- **Go-to-definition** across files
- **Dependency graph traversal** ("spider" from entry point)
- **Clean CLI** for AI agent tooling
- **Zed extension** with syntax highlighting

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

For detailed information, see [design/KNOWN_LIMITATIONS.md](design/KNOWN_LIMITATIONS.md).

## Project Structure

```
fsharp-tools/
├── crates/
│   ├── fsharp-index/    # Core library for symbol extraction and indexing
│   ├── fsharp-cli/      # Command-line tool
│   └── fsharp-lsp/      # Language server protocol implementation
└── extensions/
    └── zed-fsharp/      # Zed editor extension
```

## Installation

### From Source

```bash
cargo build --release
```

The binaries will be available at:
- `target/release/fsharp-cli`
- `target/release/fsharp-lsp`

## CLI Usage

All commands output JSON by default for easy integration with AI agents.

```bash
# Build/rebuild the index for current directory
$ fsharp-index build

# Find definition (JSON output)
$ fsharp-index def "PaymentService.processPayment"
# {"file": "src/Services.fs", "line": 42, "column": 5, ...}

# Human-readable output
$ fsharp-index def "PaymentService.processPayment" --format pretty

# Spider from entry point
$ fsharp-index spider "Program.main" --depth 5

# List all symbols matching pattern
$ fsharp-index symbols "Payment*"

# Suppress progress output (useful in scripts)
$ fsharp-index build --quiet
```

## AI Agent Integration

`fsharp-tools` is designed to be the "SQLite of code indexing" for AI agents. It provides a CLI-first, JSON-native interface that works with any agent (Claude Code, Aider, Cursor, etc.).

### CLI Contract

- **Output**: JSON by default (`--format json`). Use `--format pretty` for human-readable output.
- **Errors**: Printed to stderr in JSON format when using JSON output.
- **Exit Codes**:
    - `0`: Success
    - `1`: Not found (valid query, no results)
    - `2`: Error (invalid input, missing file, etc.)

### Claude Code Integration

Drop-in slash commands are available in `integrations/claude-code/`. Copy these to your `.claude/commands/` directory to enable:
- `/fsharp-def`: Find symbol definition
- `/fsharp-spider`: Spider dependency graph

### Claude Skills

A template for Claude Skills is available in `integrations/claude-skill/SKILL.md`.

## SQLite Storage & Type Extraction

The `fsharp-index build` command now writes every symbol, reference, and `open` directive into a SQLite database at `.fsharp-index/index.db`. This persistent store is used by both the CLI and the LSP so large workspaces can be loaded without rebuilding an in-memory index each run.

Optional semantic type data can be added by running the `scripts/extract-types.fsx` helper (automatically triggered by `fsharp-index build --extract-types`). The script uses FSharp.Compiler.Service to record function signatures and type members, which are merged into the same SQLite file for richer hover output and member-resolution heuristics.

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

## Documentation

- [Known Limitations](design/KNOWN_LIMITATIONS.md) — What this tool can and cannot do
- [Algorithmic Improvements](ALGORITHMIC_IMPROVEMENTS.md) — Planned enhancements
- [Design Document](design/fsharp-tools-design.md) — Architecture overview

## License

MIT