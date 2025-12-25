# RocketIndex 🚀
> **The Deterministic Navigation Layer for AI Agents.**

RocketIndex is a standalone, polyglot code indexer that makes AI coding assistants **faster, smarter, and more accurate** by replacing grep-based guesswork with precise, semantic code navigation.

## Three Ways to Use RocketIndex

| Interface | Best For | How It Works |
| :--- | :--- | :--- |
| **Claude Code Plugin** | Claude Code users | Install once, works everywhere automatically |
| **MCP Server** | Other AI assistants (Claude Desktop, Gemini, Zed) | Tools appear in agent's native tool list |
| **CLI Commands** | Humans, scripts, CI/CD | Shell commands with JSON output |

**Why MCP/Plugin over CLI for AI agents:** Our testing shows that when navigation tools are exposed via MCP, agents use them consistently. With CLI-only instructions (even in AGENTS.md), agents often fall back to grep patterns. [See benchmark results](#mcp-vs-cli-for-ai-agents).

## Quick Start

### For Claude Code Users

```bash
# Add the marketplace and install the plugin
/plugin marketplace add rocket-tycoon/rocket-index
/plugin install rocketindex
```

Done! The plugin downloads RocketIndex automatically. Just `cd` into any repo and ask Claude to find definitions, callers, or dependencies.

### For Other AI Assistants (Claude Desktop, Gemini, Zed)

```bash
# 1. Install RocketIndex
brew install rocket-tycoon/tap/rocket-index   # macOS
```

Then configure MCP for your tool - see [MCP Server Setup](#mcp-server-for-ai-assistants).

### For CLI / Human Use

```bash
# 1. Install RocketIndex
brew install rocket-tycoon/tap/rocket-index   # macOS

# 2. Index your project
cd /path/to/your/repo
rkt index

# 3. Keep index fresh (run in background)
rkt watch
```

See [CLI Commands](#cli-commands-for-humans--scripts) for usage.

### Other Platforms

<details>
<summary>Windows (Scoop)</summary>

```powershell
scoop bucket add rocket-tycoon https://github.com/rocket-tycoon/scoop-bucket
scoop install rocketindex
```
</details>

<details>
<summary>Linux</summary>

```bash
curl -LO https://github.com/rocket-tycoon/rocket-index/releases/latest/download/rocketindex-x86_64-unknown-linux-gnu.tar.gz
tar -xzf rocketindex-x86_64-unknown-linux-gnu.tar.gz
sudo mv rkt rocketindex-lsp /usr/local/bin/
```
</details>

## The Problem: Navigation Trade-offs

AI agents have several options for finding code, each with limitations:

1.  **Grep / Find**: Dumb text search. Noise-heavy. Confuses "User" (class) with "user" (variable). Wastes tokens reading unrelated files.
2.  **RAG (Vector Search)**: Probabilistic guessing. Good for "concepts" ("How does auth work?"), but terrible for "execution" ("Find all callers of `process_payment`"). Frequently hallucinates or misses references.
3.  **LSP (Language Servers)**: Type-aware and precise, but coordinate-based (requires file:line:col), not symbol-based. No caller analysis or dependency graphs. Requires language runtimes and can be memory-heavy.

## The Solution: Structural Determinism

RocketIndex uses **Tree-sitter** and **SQLite** to build a precise, relational graph of your code. It is **100% deterministic**.

| Feature | **RocketIndex** | **LSP** | **RAG** | **Grep** |
| :--- | :--- | :--- | :--- | :--- |
| **Method** | AST Graph | Type Analysis | Embeddings | Text Match |
| **Query model** | Symbol name | File coordinates | Natural language | Pattern |
| **Precision** | 100% (Exact) | 100% (Exact) | Probabilistic | Low (Noisy) |
| **Find callers** | ✅ Native | ❌ Refs only | ❌ | ❌ |
| **Dependency graph** | ✅ Native | ❌ | ❌ | ❌ |
| **Setup** | Single binary | Per-language | Embedding service | Built-in |
| **Best For** | Navigation & Refactoring | Type-aware edits | Exploration | Simple edits |

## RocketIndex vs Language Servers (LSP)

Some AI tools (like Claude Code) now include LSP support. When should you use RocketIndex instead?

| Capability | **RocketIndex** | **LSP** |
| :--- | :--- | :--- |
| **Query model** | Symbol name: `find_definition("Foo.bar")` | Coordinates: `goToDefinition(file, line, col)` |
| **Find callers** | ✅ Native `find_callers` tool | ❌ Only "find references" |
| **Dependency graph** | ✅ `analyze_dependencies` | ❌ Not available |
| **Setup** | Single binary, all languages | One server per language |
| **Runtime** | None (tree-sitter) | Language runtimes required |
| **Memory** | ~50MB | Varies (can be GBs) |

**Use RocketIndex when:**
- You want to query by symbol name, not file coordinates
- You need caller/callee analysis or dependency graphs
- You work across multiple languages
- You want lightweight, zero-runtime navigation

**Use LSP when:**
- You need type-aware resolution
- You want hover documentation with type signatures
- You need compiler diagnostics (errors, warnings)

They're complementary—RocketIndex for semantic queries and navigation, LSP for type-aware features.

## MCP Server for AI Assistants

RocketIndex includes an [MCP (Model Context Protocol)](https://modelcontextprotocol.io/) server that exposes code navigation tools directly to AI assistants. This is the **recommended integration method** for AI agents.

### Setup: Claude Desktop

Add to your Claude Desktop config (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

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

### Setup: Claude Code

**Recommended:** Use the [plugin](#2-setup-for-claude-code-plugin) - see Quick Start above.

**Manual MCP:** If you prefer manual configuration:

```bash
cd /path/to/your/repo
claude mcp add --transport stdio rocket-index -- rkt serve
rkt serve add .  # Register current project
```

### Setup: Gemini CLI

Run from your project root:

```bash
cd /path/to/your/repo
gemini mcp add rocket-index rkt serve
```

See [Gemini CLI MCP docs](https://geminicli.com/docs/tools/mcp-server/) for more options.

### Setup: Zed Editor

Add to your Zed settings (`~/.config/zed/settings.json`):

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

See [Zed MCP docs](https://zed.dev/docs/ai/mcp) for more options.

### Available Tools

| Tool | Purpose | Why Better Than Grep |
| :--- | :--- | :--- |
| `find_callers` | Find all call sites of a function | Grep finds text matches; this finds actual callers |
| `analyze_dependencies` | Traverse call graph (forward/reverse) | Grep cannot do graph traversal at all |
| `find_definition` | Locate where a symbol is defined | Precise location, not 17 candidates to sift |
| `find_references` | Find all usages of a symbol | Semantic matches, not text matches |
| `search_symbols` | Search symbols by pattern | Structured results with qualified names |
| `describe_project` | Get project structure overview | Semantic map, not just file listing |

### Managing Projects

```bash
rkt serve add /path/to/project     # Register a project
rkt serve remove /path/to/project  # Remove a project
rkt serve list                     # List registered projects
```

Projects are automatically indexed on first use. Configure auto-watch in `~/.config/rocketindex/mcp.json`:

```json
{
  "projects": ["/path/to/project1", "/path/to/project2"],
  "auto_watch": true,
  "debounce_ms": 200
}
```

---

## CLI Commands for Humans & Scripts

The CLI provides the same capabilities as MCP, designed for human use, shell scripts, and CI/CD pipelines.

### Navigation Commands

```bash
# Find where a symbol is defined
rkt def "User"                    # -> src/models/User.fs:12

# Find all callers of a function
rkt callers "User.save"           # -> src/controllers/AuthController.fs:45

# Find all references to a symbol
rkt refs "Config"                 # All usages across codebase
```

### Impact Analysis

```bash
# What depends on this function? (reverse dependency graph)
rkt spider "validate_email" --reverse --depth 3

# What does this function call? (forward dependency graph)
rkt spider "main" --depth 5
```

### Symbol Discovery

```bash
# Search for symbols by pattern (supports wildcards)
rkt symbols "*Service" --concise
```

### Watch Mode (Essential for AI Coding Sessions)

Keep the index fresh as files change:

```bash
# Run in a background terminal during coding sessions
rkt watch
```

Without watch mode, the index becomes stale as files are modified.

## MCP vs CLI for AI Agents

We tested whether AI agents would use RocketIndex tools when provided via CLI instructions (AGENTS.md) vs MCP server.

**Task:** "Find all functions that call ApiProviderFactory" in vets-api (7,470 Ruby files, 37K symbols)

| Model | Integration | Turns | Result |
| :--- | :--- | :--- | :--- |
| Haiku | **MCP Server** | **2** | Success (16 callers found) |
| Haiku | CLI (AGENTS.md) | 10 | Failed (max turns) |
| Sonnet | **MCP Server** | **2** | Success |
| Sonnet | CLI (AGENTS.md) | 10 | Failed (max turns) |

**Finding:** With MCP, agents immediately use `find_callers`. Without MCP, they fall back to grep patterns even when AGENTS.md explicitly documents the CLI commands. Tool visibility matters more than documentation.

---

## Performance Benchmarks

### Example: Finding `spawn` in Tokio (751 Rust files)

**Without RocketIndex** - Agent uses grep, gets 17 candidate files:
```
tokio/src/process/mod.rs:863:    pub fn spawn(&mut self) -> ...
tokio-util/src/task/task_tracker.rs:381:    pub fn spawn<F>(...
tokio-util/src/task/join_map.rs:264:    pub fn spawn<F>(...
... 14 more matches
```
Then it reads each file to verify. **3-5 tool calls, 5,000+ tokens consumed.**

**With RocketIndex** - One call, one exact answer:
```bash
$ rkt def "spawn"
# -> tokio-util/tests/task_tracker.rs:186
```
**1 tool call, 221 tokens.**

### Summary

| Metric | RocketIndex | Grep Workflow |
| :--- | :--- | :--- |
| **Tool calls** | 1 | 3-5+ |
| **Output size** | 221 chars | 5,000+ chars |
| **Precision** | Exact | 17 candidates to sift |

## Language Support

| Language | Extensions | Status |
| :--- | :--- | :--- |
| **F#** | `.fs`, `.fsi`, `.fsx` | Full |
| **Go** | `.go` | Full |
| **JavaScript** | `.js`, `.jsx`, `.mjs`, `.cjs` | Full |
| **Python** | `.py`, `.pyi` | Full |
| **Ruby** | `.rb` | Full |
| **Rust** | `.rs` | Full |
| **TypeScript** | `.ts`, `.tsx` | Full |
| **C#** | `.cs` | Beta |
| **Java** | `.java` | Beta |
| **PHP** | `.php` | Beta |
| **C** | `.c`, `.h` | Alpha |
| **C++** | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh` | Alpha |

**Status Key:**
- **Full**: Production-ready. Comprehensive symbol extraction with visibility, inheritance, and language-specific patterns.
- **Beta**: Functional for most use cases. Core symbols extracted; some advanced patterns may be missing.
- **Alpha**: Basic support. Functions, classes, and modules extracted; limited metadata.

## Security

RocketIndex uses [Tree-sitter](https://tree-sitter.github.io/) for pure syntactic analysis. In the core indexer path, source files are only ever **read as bytes and parsed into an AST**; they are never compiled or executed. Symbol information is extracted from the AST and stored in SQLite.

The CLI may optionally invoke external tools like `git` (for history) or `dotnet fsi` (for F# type extraction). These tools can execute project build scripts or code if you enable those features, so they should not be used with untrusted repositories unless you trust your toolchain and environment.

With those caveats, the default indexing and navigation features of RocketIndex are safe to run on untrusted codebases: **indexed files themselves are never executed by RocketIndex.**

## License
[BSL 1.1](LICENSE) - Source Available.