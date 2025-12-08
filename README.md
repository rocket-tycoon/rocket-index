# RocketIndex 🚀
> **The Deterministic Navigation Layer for AI Agents.**

RocketIndex is a standalone, polyglot code indexer designed to make your AI coding assistant (Claude Code, Aider, Copilot CLI) **faster, smarter, and more accurate.**

It provides a high-performance CLI that acts as an **O(1) Oracle** for your agent, allowing it to navigate massive codebases (1M+ lines) with zero hallucination and negligible token usage.

## Fast Track

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
# Download latest release (adjust version as needed)
curl -LO https://github.com/rocket-tycoon/rocket-index/releases/latest/download/rocketindex-x86_64-unknown-linux-gnu.tar.gz
tar -xzf rocketindex-x86_64-unknown-linux-gnu.tar.gz
sudo mv rkt rocketindex-lsp /usr/local/bin/
```

### 2. Start RocketIndex
Run this in a separate terminal. It will set up your agent, index your codebase, and watch for changes.

```bash
cd /path/to/your/repo
rkt start claude   # or: cursor, copilot
```

This runs `rkt setup` (installs slash commands and rules for your agent) then starts watch mode.

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

## Agent Capabilities

RocketIndex is designed for **Tool Use**. Almost all commands output concise JSON by default (or clean text for LLMs).

### 1. Unified Navigation
Whether it's F# or Ruby, the commands are the same. The agent doesn't need to know the language specifics.

```bash
# "Where is the User class defined?"
$ rkt def "User" 
# -> src/models/User.fs:12

# "Who calls the save method?"
$ rkt callers "User.save"
# -> src/controllers/AuthController.fs:45
```

### 2. Impact Analysis ("Spidering")
Before editing a function, an agent can "spider" the dependency graph to see what will break.

```bash
# "What depends on this function? Recurse 3 levels deep."
$ rkt spider "validate_email" --reverse --depth 3
```

### 3. Symbol Discovery
Agents can quickly map out a new codebase without reading file contents.

```bash
# "What services do we have?"
$ rkt symbols "*Service" --concise
```

## Performance (Real-World Benchmarks)

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

### Benchmarks

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
