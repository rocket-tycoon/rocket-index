# RocketIndex 🚀

**The Deterministic Navigation Layer for AI Agents**

One call. Exact answer.

[![License: BSL 1.1](https://img.shields.io/badge/License-BSL%201.1-blue.svg)](LICENSE)
[![Latest Release](https://img.shields.io/github/v/release/rocket-tycoon/rocket-index)](https://github.com/rocket-tycoon/rocket-index/releases)

---

## Why RocketIndex?

AI coding agents waste tokens because they navigate code by guessing. Grep returns 17 candidates when you need one. LSPs require file coordinates when agents only have a symbol name. Both force agents into multi-turn loops.

RocketIndex replaces guesswork with structure. It parses your code into an AST using Tree-sitter, stores the symbol graph in SQLite, and answers navigation queries deterministically.

### vs Grep: Structure beats text matching

**Example: Finding `spawn` in Tokio (751 Rust files)**

```bash
$ rkt def "spawn"
# -> tokio/src/runtime/blocking/pool.rs:57
```

| Metric | RocketIndex | Grep |
|--------|-------------|------|
| Tool calls | 1 | 3-5+ |
| Tokens | ~200 | 5,000+ |
| Result | Exact definition | 17 candidates to verify |

Grep matches text. RocketIndex understands code structure—it knows the difference between a definition, a call site, and a comment.

### vs LSPs: Query by name, not coordinates

LSPs are designed for editors where a cursor is already positioned on a symbol. AI agents don't have cursors—they have symbol names from user requests.

| Task | RocketIndex | LSP |
|------|-------------|-----|
| "Find `UserService.save`" | `find_definition("UserService.save")` | Find file containing text → position cursor → goToDefinition |
| "Who calls this?" | `find_callers("save")` | Only "find references" (callers + callees mixed) |
| "Trace the call graph" | `analyze_dependencies("main")` | Not available |

LSPs also require language runtimes and often fail on incomplete code. RocketIndex uses pure syntactic analysis—it works on partial checkouts, broken builds, and across 15 languages with a single binary.

---

## How It Works

```
Source Files → Tree-sitter → AST → SQLite → Deterministic Queries
```

| Capability | RocketIndex | LSP | Vector Search | Grep |
|------------|-------------|-----|---------------|------|
| Query model | Symbol name | File coordinates | Natural language | Pattern |
| Find callers | Yes | Refs only | No | No |
| Dependency graph | Yes | No | No | No |
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

They're complementary. Use both.

**Use RocketIndex for:** Navigation by symbol name, caller/callee analysis, dependency graphs, multi-language codebases, partial checkouts.

**Use LSP for:** Type-aware resolution, hover documentation, compiler diagnostics, refactoring with type safety.

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
| Kotlin | Full | `.kt`, `.kts` | Classes, Objects, Interfaces, Functions, Properties |
| PHP | Full | `.php` | |
| Python | Full | `.py`, `.pyi` | |
| Ruby | Full | `.rb` | |
| Rust | Full | `.rs` | |
| Swift | Full | `.swift` | Classes, Structs, Enums, Protocols, Functions, Properties |
| TypeScript | Full | `.ts`, `.tsx` | |
| Objective-C | Full | `.m`, `.mm` | Classes, Protocols, Methods, Properties, Categories |
| Haxe | Beta | `.hx` | Classes, Interfaces, Typedefs, Functions, Variables, Metadata |

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
