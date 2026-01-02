# RocketIndex 🚀

**The Deterministic Navigation Layer for AI Agents**

One call. Exact answer.

[![License: BSL 1.1](https://img.shields.io/badge/License-BSL%201.1-blue.svg)](LICENSE)
[![Latest Release](https://img.shields.io/github/v/release/rocket-tycoon/rocket-index)](https://github.com/rocket-tycoon/rocket-index/releases)

---

## Why RocketIndex?

AI coding agents waste tokens because they navigate code by guessing. Grep returns 17 candidates when you need one. Vector search hallucinates. Both force agents into multi-turn loops reading files to verify what they found.

RocketIndex replaces guesswork with structure. It parses your code into an AST using Tree-sitter, stores the symbol graph in SQLite, and answers navigation queries deterministically.

**Example: Finding `spawn` in Tokio (751 Rust files)**

Without RocketIndex, grep returns 17 candidates. The agent reads each file to verify—3-5 tool calls, 5,000+ tokens consumed.

With RocketIndex:
```bash
$ rkt def "spawn"
# -> tokio/src/runtime/blocking/pool.rs:57
```

One call. 200 tokens. Exact answer.

| Metric | RocketIndex | Grep Workflow |
|--------|-------------|---------------|
| Tool calls | 1 | 3-5+ |
| Tokens | ~200 | 5,000+ |
| Result | Exact definition | 17 candidates to sift |

---

## How It Works

```
Source Files → Tree-sitter → AST → SQLite → Deterministic Queries
```

| Capability | RocketIndex | LSP | Vector Search | Grep |
|------------|-------------|-----|---------------|------|
| Query model | Symbol name | File coordinates | Natural language | Pattern |
| Find callers | ✅ Native | ❌ Refs only | ❌ | ❌ |
| Dependency graph | ✅ Native | ❌ | ❌ | ❌ |
| Precision | Deterministic | Deterministic | Probabilistic | Noisy |
| Best for | Navigation, refactoring | Type-aware edits | Exploration | Simple edits |

---
 
 ## Built for Scale
 
 Rocket Index was built specifically because traditional Language Servers often choke on large codebases.
 
 *   **Zero Compilation**: Unlike LSPs that attempt to compile your project (which fails on partial checkouts or missing dependencies), Rocket Index uses **pure syntactic analysis**. It doesn't care if your code compiles, only that it parses.
 *   **Monorepo Ready**: Designed to handle repositories with **10,000+ files** without breaking a sweat.
 *   **Instant Access**: Uses an embedded **SQLite** database with aggressive performance tuning (WAL mode, memory-mapped I/O) to ensure symbol lookups take milliseconds, regardless of project size.
 *   **Low Footprint**: Keeps your agent's context lightweight. Instead of loading the entire project structure into memory, it performs targeted disk-based queries.
 
 ---

## Quick Start

### Install

**macOS**
```bash
brew install rocket-tycoon/tap/rocket-index
```

**Windows**
```powershell
scoop bucket add rocket-tycoon https://github.com/rocket-tycoon/scoop-bucket
scoop install rocketindex
```

**Linux**
```bash
curl -LO https://github.com/rocket-tycoon/rocket-index/releases/latest/download/rocketindex-x86_64-unknown-linux-gnu.tar.gz
tar -xzf rocketindex-x86_64-unknown-linux-gnu.tar.gz
sudo mv rkt rocketindex-lsp /usr/local/bin/
```

### For AI Assistants (Recommended)

RocketIndex exposes tools via [MCP (Model Context Protocol)](https://modelcontextprotocol.io/). This is the recommended integration—our testing shows agents use navigation tools consistently when exposed via MCP, but fall back to grep when given CLI instructions.

The MCP server auto-indexes on first use. No manual setup required.

#### Claude Code

```bash
# Add the plugin (recommended)
/plugin marketplace add rocket-tycoon/rocket-index
/plugin install rocketindex
```

Or configure MCP manually:

```bash
cd /path/to/your/repo
claude mcp add --transport stdio rocket-index -- rkt serve
```

#### Claude Desktop

Add to your config file:

| Platform | Config Path |
|----------|-------------|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

```json
{
  "mcpServers": {
    "rocket-index": {
      "command": "rkt",
      "args": ["serve"]
    }
  }
}
```

Restart Claude Desktop. RocketIndex tools will appear in the toolbox.

#### Other AI Tools

**Gemini CLI:**
```bash
gemini mcp add rocket-index rkt serve
```

**Zed Editor** — add to `~/.config/zed/settings.json`:
```json
{
  "context_servers": {
    "rocket-index": {
      "command": "rkt",
      "args": ["serve"]
    }
  }
}
```

### For Humans and Scripts

```bash
cd /path/to/your/repo
rkt index                    # Build index
rkt watch                    # Keep index fresh (run in background terminal)
```

**Important:** Run `rkt watch` in a background terminal during coding sessions. Without it, the index becomes stale as files change.

---

## What It Does

### MCP Tools (for AI agents)

| Tool | Purpose |
|------|---------|
| `find_definition` | Locate where a symbol is defined |
| `find_callers` | Find all call sites of a function |
| `find_references` | Find all usages of a symbol |
| `analyze_dependencies` | Traverse call graph forward or reverse |
| `search_symbols` | Search symbols by pattern |
| `describe_project` | Get semantic project structure |

### CLI Commands (for humans)

```bash
rkt def "User"                          # Find definition
rkt callers "User.save"                 # Find all callers
rkt refs "Config"                       # Find all references
rkt spider "validate_email" --reverse   # Reverse dependency graph
rkt symbols "*Service" --concise        # Search by pattern
rkt blame "UserService.save"            # Blame by symbol (or file:line)
```

---

## RocketIndex vs Language Servers

Some AI tools (like Claude Code) include LSP support. When should you use RocketIndex instead?

| Capability | RocketIndex | LSP |
|------------|-------------|-----|
| Query model | Symbol name: `find_definition("Foo.bar")` | Coordinates: `goToDefinition(file, line, col)` |
| Find callers | ✅ Native | ❌ Only "find references" |
| Dependency graph | ✅ `analyze_dependencies` | ❌ Not available |
| Setup | Single binary, all languages | One server per language |
| Runtime | None (tree-sitter) | Language runtimes required |

**Use RocketIndex when:**
- You want to query by symbol name, not file coordinates
- You need caller/callee analysis or dependency graphs
- You work across multiple languages

**Use LSP when:**
- You need type-aware resolution
- You want hover documentation with type signatures
- You need compiler diagnostics

They're complementary—RocketIndex for structural navigation, LSP for type-aware features.

---

## Language Support


| Language | Status | Extensions | Features |
|----------|--------|------------|----------|
| C | Full | `.c`, `.h` | Structs, Unions, Enums, Typedefs, Functions |
| C# | Full | `.cs` | |
| C++ | Full | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh` | Namespaces, Classes, Inheritance, Templates |
| F# | Full | `.fs`, `.fsi`, `.fsx` | |
| Go | Full | `.go` | |
| Java | Full | `.java` | |
| JavaScript | Full | `.js`, `.jsx`, `.mjs`, `.cjs` | |
| PHP | Full | `.php` | |
| Python | Full | `.py`, `.pyi` | |
| Ruby | Full | `.rb` | |
| Rust | Full | `.rs` | |
| Swift | Full | `.swift` | Classes, Structs, Enums, Protocols, Functions, Properties |
| TypeScript | Full | `.ts`, `.tsx` | |
| Haxe | Beta | `.hx` | Classes, Interfaces, Typedefs, Functions, Variables, Metadata |
| Kotlin | Beta | `.kt`, `.kts` | Classes, Objects, Interfaces, Functions, Properties |
| Objective-C | Beta | `.m`, `.mm` | Classes, Protocols, Methods, Properties, Categories |

**Full:** Production-ready with visibility, inheritance, and language-specific patterns.
**Beta:** Core symbols extracted; some advanced patterns may be missing.
**Alpha:** Basic function/class/module extraction.

---

## Configuration

### MCP Project Management

```bash
rkt serve add /path/to/project     # Register project
rkt serve remove /path/to/project  # Remove project
rkt serve list                     # List projects
```

### Auto-watch Configuration

Create `~/.config/rocketindex/mcp.json`:
```json
{
  "projects": ["/path/to/project1", "/path/to/project2"],
  "auto_watch": true,
  "debounce_ms": 200
}
```

---

## Troubleshooting

**Index out of date?**
Run `rkt watch` in a background terminal during coding sessions, or re-run `rkt index` after significant changes.

**MCP tools not appearing?**
Restart your editor/agent after adding the MCP configuration. Verify with `rkt serve list` that your project is registered.

**Symbols missing?**
Check language support status above. For Alpha/Beta languages, some symbol types may not be extracted. File an issue with a minimal reproduction.

**Large monorepo slow to index?**
Initial indexing is I/O bound. Subsequent incremental updates via `rkt watch` are fast. Consider indexing specific subdirectories if you only work in part of the repo.

---

## Security

RocketIndex uses Tree-sitter for pure syntactic analysis. Source files are read as bytes, parsed into an AST, and symbol information is stored in SQLite. **Indexed files are never compiled or executed.**

Optional features like git blame or F# type extraction invoke external tools (`git`, `dotnet fsi`) which may execute project code. Use caution with untrusted repositories when these features are enabled.

---

## License

[BSL 1.1](LICENSE) — Source available. See license for usage terms.

---

## Links

- [Releases & Changelog](https://github.com/rocket-tycoon/rocket-index/releases)
- [Contributing](CONTRIBUTING.md)
- [Coding Guidelines](coding-guidelines.md)
