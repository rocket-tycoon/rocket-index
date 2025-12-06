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

## Performance

RocketIndex is **10-100x faster** than traditional text search tools for code navigation queries.

### Benchmarks (vets-api: 7,434 files, 36,286 symbols)

| Operation | RocketIndex | grep | ripgrep | Speedup |
|-----------|-------------|------|---------|---------|
| **Build Index** | 2.65s | N/A | N/A | One-time cost |
| **Subclasses Query** | 0.009-0.038s | 1.1-1.3s | 0.25s | **10-100x** |
| **Definition Lookup** | 0.010s | 1.2s | 0.015-0.033s | **100x** vs grep |
| **Symbol Search** | 0.012-0.025s | 1.1-1.3s | 0.25s | **10-100x** |

### Semantic Search vs String Search

String search tools get confused by comments, strings, and naming collisions. RocketIndex understands code structure.

**Summary: Querying "User" in vets-api (7,434 Ruby files)**

| Query | RocketIndex | grep | Notes |
|-------|-------------|------|-------|
| Subclasses of `User` | **1** (exact) | 8 matches | grep includes `UserIdentity`, `UserAccountError`, test mocks |
| Methods on `User` | **80** (exact) | 859 matches | grep includes comments, other classes, unrelated code |
| Definition of `User` | **1** (exact) | 60 matches | grep matches `UserIdentity`, `UserAccount`, etc. |
| Raw "User" mentions | N/A | 2,339 matches | String search - grep's domain |

**Example: "Find all subclasses of User"**

```bash
# RocketIndex: 0.02s - returns exactly 1 result
$ rkt subclasses "User"
IAMUser (app/models/iam_user.rb:11)

# grep: 1.3s - returns 8 results (7 false positives!)
$ grep -r "< User" --include="*.rb" .
./app/models/iam_user.rb:class IAMUser < User                    # ✓ correct
./app/models/iam_user_identity.rb:class IAMUserIdentity < UserIdentity  # ✗ different class
./app/services/mhv/user_account/errors.rb:class CreatorError < UserAccountError  # ✗ different class
# ... plus 5 more false positives
```

**Example: "Find all methods on the User class"**

```bash
# RocketIndex: 0.02s - returns exactly 80 methods
$ rkt symbols "User#*"
User#initial_sign_in    app/models/user.rb:36
User#credential_lock    app/models/user.rb:40
# ... 78 more

# grep: 1.5s - returns 859 lines of noise
$ grep -ri "def.*user" --include="*.rb" .
# Returns methods from UserAccount, UserIdentity, comments, specs...
```

Run `./scripts/benchmark_semantic.sh` to reproduce these benchmarks on your own codebase.

### Why So Fast?

*   **Pre-computed Index**: Symbols are parsed once and stored in SQLite with optimized indexes.
*   **O(1) Lookups**: Definition queries use indexed lookups, not full-text scans.
*   **Parallel Parsing**: Uses Rayon for multi-threaded file parsing during index builds.
*   **SQLite Tuning**: WAL mode, memory-mapped I/O, and aggressive caching for sub-millisecond queries.

### Memory Usage

| Tool | Memory (vets-api) |
|------|-------------------|
| RocketIndex | ~50MB |
| Traditional LSP | 500MB-2GB+ |
| grep/ripgrep | Minimal (streaming) |

RocketIndex achieves LSP-quality results with grep-like memory efficiency.

## Symbol Metadata

RocketIndex extracts rich metadata from your code, enabling powerful queries beyond simple text search.

| Field | Description | Example |
|-------|-------------|---------|
| `parent` | Parent class/type for inheritance | `Dog` inherits from `Animal` |
| `mixins` | Ruby include/extend/prepend modules | `include Enumerable` |
| `attributes` | Decorators and annotations | F# `[<Obsolete>]`, Ruby `attr_accessor` |
| `implements` | Interface implementations | F# `interface IComparable` |
| `doc` | Documentation comments | F# `///`, Ruby `#` |
| `signature` | Type signatures | `int -> int -> int` |

### Example Queries

```bash
# Find all classes that inherit from a base class
$ rkt subclasses "Common::Client::Base"

# Symbol search includes metadata in JSON output
$ rkt def "PaymentService" --json
# Returns: { "name": "PaymentService", "parent": "BaseService", "implements": ["IPayable"], ... }
```

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
$ rkt index

# Find definition (with optional git provenance)
$ rkt def "PaymentService.processPayment"
$ rkt def "PaymentService.processPayment" --git  # Include author, date, commit

# Search symbols (supports wildcards)
$ rkt symbols "User*"
$ rkt symbols "process*" --concise

# Find all uses of a symbol (cross-reference search)
$ rkt refs --symbol "PaymentService.process"
$ rkt refs --symbol "User" --context 2  # Show 2 lines of context

# Find references in a file
$ rkt refs --file "src/Services.fs"
```

### Dependency Analysis

```bash
# Spider forward: what does this symbol depend on?
$ rkt spider "Program.main" -d 5

# Spider reverse: what depends on this symbol? (impact analysis)
$ rkt spider "validateInput" --reverse -d 3

# Find direct callers (single-level reverse spider)
$ rkt callers "PaymentService.processPayment"
```
### Git Integration

```bash
# Git blame for a specific line
$ rkt blame "src/Services.fs:42"

# Git history for a symbol
$ rkt history "PaymentService"
```

### Diagnostics

```bash
# Check index health
$ rkt doctor

# Set up editor integrations
$ rkt setup claude  # Interactive skill selection
$ rkt setup cursor  # Creates .cursor/rules
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

Binaries will be at `target/release/rkt` (CLI) and `target/release/rocketindex-lsp` (Server).

## Agent Integration Guide

### Claude Code

The fastest way to get started:

```bash
rkt setup claude
```

- **Agent Instructions**: Configures `CLAUDE.md` and creates `.rocketindex/AGENTS.md` for context.
- **Skills library** (optional) - 5 role-based prompts plus RocketIndex:
  - **RocketIndex** - Code navigation (always installed)
  - **Lead Engineer** - Design, implementation, ADRs
  - **QA Engineer** - Testing, verification
  - **SRE** - Reliability, performance, stacktrace analysis
  - **Security Engineer** - Vulnerability analysis
  - **Product Manager** - Requirements, acceptance criteria

Each skill includes bounded checklists and integrates RocketIndex commands for code navigation.

### Cursor

```bash
rkt setup cursor
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
./target/release/rkt index

# 3. Check health
./target/release/rkt doctor

# 4. Set up your editor (optional)
./target/release/rkt setup claude

# 5. Start exploring
./target/release/rkt symbols "*Service*" --concise
./target/release/rkt def "MyModule.myFunction" --git
./target/release/rkt callers "validateInput"
```

## License

[BSL 1.1](LICENSE) - Copyright (c) 2025 Alastair Dawson

This is source-available software under the Business Source License 1.1. It converts to Apache 2.0 after four years. See LICENSE for details on permitted uses.
