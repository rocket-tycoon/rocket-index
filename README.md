# RocketIndex

**The LSP for the AI Era.**

RocketIndex is a rocket-fast, polyglot language server and code indexer designed for the modern development workflow: **Humans architecting solutions with AI Agents.**

Written in pure Rust with ~50MB RAM usage. No runtime required.

## The Shift: Developer as Architect

The role of the software engineer is evolving. You are no longer just a coder; you are a Technical Product Manager and Solution Architect for a team of AI agents. You design the system, and your agents (Claude Code, Aider, etc.) implement and refactor it.

But agents are blind. They run in headless terminals. They can't "right-click -> Go to Definition". They waste massive amounts of context tokens reading unrelated files just to find where a class is defined.

**RocketIndex bridges this gap.** It gives your agents the same code intelligence you have in your IDE, but via a high-performance CLI.

## Why RocketIndex?

### 🤖 For Your Agents (The Brain)
RocketIndex is the "LSP for Agents". It provides a structured, queryable interface to your codebase.

*   **Headless Intelligence**: Agents can ask *"Where is `User` defined?"* or *"Who calls `process_payment`?"* and get an instant, precise answer.
*   **Context Efficiency**: Instead of `grep`-ing or reading 50 files (wasting tokens and money), agents get the exact file and line number in milliseconds.
*   **Unified Tooling**: Whether your agent is working on F# or Ruby, it uses the exact same commands (`def`, `refs`, `symbols`).

### 🧑‍💻 For You (The Eyes)
RocketIndex powers the **"F# Fast"** and **"Ruby Fast"** extensions for Zed.

*   **Instant Feedback**: Go-to-definition and symbol search that feels instantaneous.
*   **Lightweight**: Uses <50MB RAM, unlike other servers that eat GBs of memory.
*   **Reliable**: Built on SQLite and Rust. It doesn't crash on large monoliths.

## Key Differentiators

RocketIndex was built to overcome specific limitations of existing tools:

1.  **Unbounded Memory Growth**: Unlike some language servers that load the entire project state into RAM (causing crashes), RocketIndex uses streaming parsing and a SQLite backend to keep memory usage low and constant.
2.  **File Count Limits**: Unlike servers that cap indexing at ~5000 files, RocketIndex handles large monoliths (like `vets-api` with 7000+ files) with ease.
3.  **Polyglot Core**: Powered by Tree-sitter, allowing it to support multiple languages (currently F# and Ruby) with a single binary.

## Language Support

| Language | Extensions | Features |
|----------|------------|----------|
| **F#** | `.fs`, `.fsi`, `.fsx` | Modules, functions, types, records, unions, interfaces |
| **Ruby** | `.rb` | Classes, modules, methods, constants |

Additional languages can be added via Tree-sitter grammars.

## Why not MCP?

While the Model Context Protocol (MCP) is great for connecting agents to static resources (Jira, Postgres), it is the "wrong abstraction" for deep code intelligence.

1.  **Context Bloat**: MCP forces you to load tool definitions upfront. For a large codebase, this "dictionary problem" wastes thousands of tokens and makes agents "dumber" due to cognitive overload. RocketIndex acts as an O(1) Oracle, answering specific queries without polluting the context.
2.  **Statelessness**: MCP is designed to be stateless. RocketIndex uses a persistent SQLite graph, allowing for instant, complex "spidering" of dependency trees that would be impossibly slow over a stateless JSON-RPC protocol.
3.  **Domain Specificity**: MCP treats code as generic "resources". RocketIndex understands the *graph* of code (definitions, references, calls), providing "spatial reasoning" that generic tools cannot match.

## CLI Usage (for Agents)

All commands output JSON by default for easy integration. Use `--concise` for minimal output (saves tokens).

### Core Commands

```bash
# Index the codebase (run first!)
$ rocketindex index

# Find definition (with optional git provenance)
$ rocketindex def "PaymentService.processPayment"
$ rocketindex def "PaymentService.processPayment" --git  # Include author, date, commit

# Search symbols (supports wildcards)
$ rocketindex symbols "User*"
$ rocketindex symbols "process*" --concise

# Find references in a file
$ rocketindex refs "src/Services.fs"
```

### Dependency Analysis

```bash
# Spider forward: what does this symbol depend on?
$ rocketindex spider "Program.main" -d 5

# Spider reverse: what depends on this symbol? (impact analysis)
$ rocketindex spider "validateInput" --reverse -d 3

# Find direct callers (single-level reverse spider)
$ rocketindex callers "PaymentService.processPayment"
```

### Git Integration

```bash
# Git blame for a specific line
$ rocketindex blame "src/Services.fs:42"

# Git history for a symbol
$ rocketindex history "PaymentService"
```

### Diagnostics

```bash
# Check index health
$ rocketindex doctor

# Set up editor integrations
$ rocketindex setup claude  # Interactive skill selection
$ rocketindex setup cursor  # Creates .cursor/rules
```

### Output Modes

| Flag | Description |
|------|-------------|
| `--format json` | Machine-readable JSON (default) |
| `--format pretty` | Human-readable with colors |
| `--concise` | Minimal fields, saves tokens |
| `--quiet` | Suppress progress output |

## IDE Integration (for Humans)

RocketIndex powers the following Zed extensions:
*   **F# Fast** (`extensions/zed-fsharp`)
*   **Ruby Fast** (`extensions/zed-ruby`)

These provide syntax highlighting, go-to-definition, and symbol search within the editor.

## Installation

### From Source

```bash
cargo build --release
```

Binaries will be at `target/release/rocketindex` (CLI) and `target/release/rocketindex-lsp` (Server).

## Agent Integration Guide

### Claude Code

The fastest way to get started:

```bash
rocketindex setup claude
```

This creates:
- `/ri` slash command for quick access to RocketIndex commands
- **Skills library** (optional) - 9 role-based prompts that help Claude frame tasks:
  - **Tech Lead** - Task breakdown, code review, architectural oversight
  - **Architect** - System design, technical decisions, ADRs
  - **Developer** - Implementation with codebase conventions
  - **QA Engineer** - Testing, verification, coverage
  - **Product Manager** - Requirements, user stories, acceptance criteria
  - **Performance Engineer** - Optimization, benchmarking, profiling
  - **Security Engineer** - Vulnerability analysis, OWASP review
  - **SRE** - Observability, reliability, error handling
  - **RocketIndex** - Code navigation specialist

Each skill includes bounded checklists and integrates RocketIndex commands for code navigation.

### Cursor

```bash
rocketindex setup cursor
```

Creates `.cursor/rules` with RocketIndex guidance.

### Custom Agents

RocketIndex follows a simple contract:

| Aspect | Details |
|--------|---------|
| **Output** | JSON (`--format json`) or Human (`--format pretty`) |
| **Exit Codes** | `0` (Success), `1` (Not Found), `2` (Error) |
| **Storage** | `.rocketindex/index.db` (SQLite) |
| **Token Efficiency** | Use `--concise` for minimal output |

## Limitations

*   **Syntactic Analysis Only**: No compiler/runtime required. This means no type inference or overload resolution.
*   **No External Indexing**: Standard libraries and external packages (NuGets, Gems) are not indexed.
*   **Approximate Resolution**: Spider dependency graphs are best-effort based on syntactic matching.

## Quick Start

```bash
# 1. Build from source
cargo build --release

# 2. Index your codebase
./target/release/rocketindex index

# 3. Check health
./target/release/rocketindex doctor

# 4. Set up your editor (optional)
./target/release/rocketindex setup claude

# 5. Start exploring
./target/release/rocketindex symbols "*Service*" --concise
./target/release/rocketindex def "MyModule.myFunction" --git
./target/release/rocketindex callers "validateInput"
```

## License

[MIT](LICENSE) - Copyright (c) 2025 Alastair Dawson
