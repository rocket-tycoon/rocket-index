# RocketIndex 🚀
> **The Deterministic Navigation Layer for AI Agents.**

RocketIndex is a standalone, polyglot code indexer that makes AI coding assistants **faster, smarter, and more accurate** by replacing grep-based guesswork with precise, semantic code navigation.

## Two Ways to Use RocketIndex

| Interface | Best For | How It Works |
| :--- | :--- | :--- |
| **MCP Server** (Recommended for AI) | AI assistants (Claude, etc.) | Tools appear in agent's native tool list |
| **CLI Commands** | Humans, scripts, CI/CD | Shell commands with JSON output |

**Why MCP is recommended for AI agents:** Our testing shows that when navigation tools are exposed via MCP, agents use them consistently. With CLI-only instructions (even in AGENTS.md), agents often fall back to grep patterns. [See benchmark results](#mcp-vs-cli-for-ai-agents).

## Quick Start

### 1. Install

**macOS (Homebrew)**
```bash
brew install rocket-tycoon/tap/rocket-index
```

**Windows (Scoop)**
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

### 2. Choose Your Setup Path

**For AI Assistants (MCP) - Recommended**
```bash
cd /path/to/your/repo
claude mcp add --transport stdio rocket-index -- rkt serve
```
That's it. The MCP server auto-indexes on first use. See [MCP Setup](#mcp-server-for-ai-assistants) for Claude Desktop config.

**For Human Use / CLI**
```bash
cd /path/to/your/repo
rkt index                    # Build index
rkt watch                    # Keep index fresh (run in background terminal)
```

## The Problem: Approximate Navigation

Agents today rely on two flawed methods for finding code:

1.  **Grep / Find**: Dumb text search. Noise-heavy. Confuses "User" (class) with "user" (variable). Wastes tokens reading unrelated files.
2.  **RAG (Vector Search)**: Probabilistic guessing. Good for "concepts" ("How does auth work?"), but terrible for "execution" ("Find all callers of `process_payment`"). It frequently hallucinates or misses references.

## The Solution: Structural Determinism

RocketIndex uses **Tree-sitter** and **SQLite** to build a precise, relational graph of your code. It is **100% deterministic**.

| Feature | **RocketIndex** | **Vector Search (RAG)** | **Grep** |
| :--- | :--- | :--- | :--- |
| **Method** | **AST Graph** | Embeddings | Text Match |
| **Precision** | **100% (Exact)** | Probabilistic | Low (Noisy) |
| **Latency** | **< 10ms** | ~200ms+ | Variable |
| **Best For** | **Navigation & Refactoring** | Exploration | Simple edits |

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

Run from your project root (RocketIndex MCP is project-local):

```bash
cd /path/to/your/repo
claude mcp add --transport stdio rocket-index -- rkt serve
```

This registers the MCP server for this project. Claude Code will auto-start it when you open the project.

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
| `enrich_symbol` | Get comprehensive symbol context | Aggregates definition, callers, blame in one call |
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

# Get comprehensive info about a symbol
rkt enrich "PaymentService.process"
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