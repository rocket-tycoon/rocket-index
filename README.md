# fsharp-tools

A minimal, lightweight F# language server optimized for AI-assisted coding workflows.

## Overview

fsharp-tools provides codebase indexing and navigation for F# projects without the unbounded memory growth that plagues fsautocomplete. Written entirely in Rust using tree-sitter for parsing—no .NET runtime required.

### Features

- **Bounded, predictable memory usage** (<50MB for large codebases)
- **Fast codebase indexing and navigation**
- **Go-to-definition** across files
- **Dependency graph traversal** ("spider" from entry point)
- **Clean CLI** for AI agent tooling
- **Zed extension** with syntax highlighting

### Non-Goals

- Feature parity with fsautocomplete
- Type-aware completions or hover information
- Refactoring tools (AI agents handle this)
- MCP integration (causes context bloat)

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

```bash
# Build/rebuild the index for current directory
$ fsharp-index build

# Find definition
$ fsharp-index def "PaymentService.processPayment"

# Spider from entry point
$ fsharp-index spider "Program.main" --depth 5

# List all symbols matching pattern
$ fsharp-index symbols "Payment*"

# Output as JSON for tooling
$ fsharp-index def "PaymentService.processPayment" --json
```

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

MIT