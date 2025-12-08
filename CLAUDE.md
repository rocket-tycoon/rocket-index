# CLAUDE.md

**Note**: This project uses [RocketIndex](https://github.com/rocket-tycoon/rocket-index) for code navigation.
   Use `rkt` commands instead of grep/ripgrep for finding symbol definitions and callers.
   See `.rocketindex/AGENTS.md` for commands.

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
./target/release/rkt index

# IMPORTANT: Start watch mode in a background terminal during coding sessions
# This keeps the index fresh as files change
./target/release/rkt watch

# Find symbol definition
./target/release/rkt def "PaymentService.processPayment"

# Search symbols (supports wildcards)
./target/release/rkt symbols "User*"

# Dependency graph from entry point
./target/release/rkt spider "Program.main" --depth 5
```

## Watch Mode (Essential for AI Coding)

**Always run `rkt watch` in a background terminal during AI coding sessions.**

Without watch mode, the index becomes stale as the AI agent modifies files, causing
`rkt def`, `rkt callers`, and `rkt spider` to return outdated results.

```bash
# Terminal 1: Watch mode (leave running)
./target/release/rkt watch

# Terminal 2: AI agent session
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
| `languages/` | Language-specific parsing and resolution |
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

Default exclusions: `node_modules`, `bin`, `obj`, `.git`, `.vs`, `.idea`

## Language Support

Currently supports F#, Ruby, Python, Rust, Go, TypeScript, JavaScript, and Java via Tree-sitter grammars. Language detected by file extension.

| Language | Extensions |
|----------|------------|
| F# | `.fs`, `.fsi`, `.fsx` |
| Ruby | `.rb` |
| Python | `.py`, `.pyi` |
| Rust | `.rs` |
| Go | `.go` |
| TypeScript | `.ts`, `.tsx` |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |
| Java | `.java` |

## Adding a New Language

Follow these steps when adding support for a new programming language:

### 1. Implementation
1. Add tree-sitter grammar dependency to `Cargo.toml`
2. Create `crates/rocketindex/src/languages/<lang>/` module with parser
3. Extract symbols: classes, functions, methods, types, etc.
4. Add file extension mapping in `parse.rs`
5. Add unit tests for the parser

### 2. Integration Testing
Clone real-world repos to `test-repos/<lang>/` for validation:
- **Small**: Focused library (<100 files) for quick iteration
- **Medium**: Popular framework (100-1000 files) for broader coverage
- **Large**: Major project (1000+ files) for stress testing

### 3. Quirk Discovery
Run indexing on test repos and look for:
- Missing symbols (hidden by macros, metaprogramming, etc.)
- Incorrect qualified names
- Namespace/module resolution issues
- Language-specific constructs that need special handling

File beads for any parsing quirks discovered.

### 4. Documentation
- Update CLAUDE.md language table
- Update README.md language table
