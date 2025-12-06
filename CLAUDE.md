# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

**Note**: This project uses [bd (beads)](https://github.com/steveyegge/beads)
   for issue tracking. Use `bd` commands instead of markdown TODOs.
   See .rocketindex/AGENTS.md for workflow details.
   
## Coding Guidelines

Follow the Rust best practices in `coding-guidelines.md`
   
## Build & Test Commands

```bash
# Build
cargo build --release           # Release build (binaries in target/release/)
cargo build                     # Debug build

# Test
cargo test --all                # Run all tests (unit + integration)
cargo test -p rocketindex       # Test only the core crate
cargo test test_name            # Run a specific test

# Lint
cargo clippy                    # Run lints
cargo fmt                       # Format code
```

## CLI Usage

```bash
# Build index (creates .rocketindex/index.db)
./target/release/rkt build

# Find symbol definition
./target/release/rkt def "PaymentService.processPayment"

# Search symbols (supports wildcards)
./target/release/rkt symbols "User*"

# Dependency graph from entry point
./target/release/rkt spider "Program.main" --depth 5

# Watch mode (auto-reindex on file changes)
./target/release/rkt watch
```

## Architecture

### Crate Structure

```
crates/
├── rocketindex/          # Core library - parsing, indexing, resolution
├── rocketindex-cli/      # CLI binary (rkt command)
└── rocketindex-lsp/      # Language server (tower-lsp based)
```

### Core Library (`rocketindex`)

| Module | Purpose |
|--------|---------|
| `parse.rs` | Tree-sitter symbol extraction from source |
| `index.rs` | In-memory `CodeIndex` for fast symbol lookup |
| `db.rs` | SQLite persistence (`SqliteIndex`) |
| `resolve.rs` | Name resolution with scope rules and `open` statements |
| `spider.rs` | Dependency graph traversal |
| `languages/` | Language-specific resolution (F#, Ruby) |
| `config.rs` | `.rocketindex.toml` configuration loading |
| `type_cache.rs` | Optional type information from `dotnet fsi` |

### Data Flow

1. **Build**: `extract_symbols()` → Tree-sitter parsing → `Symbol` structs
2. **Store**: `SqliteIndex::insert_symbols()` → `.rocketindex/index.db`
3. **Query**: `SqliteIndex` for persistence, `CodeIndex` for resolution
4. **Resolve**: `CodeIndex::resolve()` → language-specific resolver → `ResolveResult`

### Key Types

- `Symbol`: Definition with name, qualified name, kind, location
- `Reference`: Usage of a symbol (identifier, not definition)
- `CodeIndex`: In-memory index with symbol maps and resolution logic
- `SqliteIndex`: Persistent storage with O(log n) indexed lookups
- `ResolveResult`: Resolution outcome with `ResolutionPath` explaining how found

### Storage

- Index stored in `.rocketindex/index.db` (SQLite)
- Paths stored relative to workspace root for portability
- WAL mode enabled for concurrent read access

## Configuration

Create `.rocketindex.toml` in project root:

```toml
exclude_dirs = ["vendor", "generated"]  # Additional exclusions
max_recursion_depth = 1000              # For deeply nested code (default: 500)
```

Default exclusions: `node_modules`, `bin`, `obj`, `packages`, `.git`, `.vs`, `.idea`

## Language Support

Currently supports F# and Ruby via Tree-sitter grammars. Language detected by file extension.