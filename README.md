# RocketIndex 🚀
> **The Deterministic Navigation Layer for AI Agents.**

RocketIndex is a standalone, polyglot code indexer designed to make your AI coding assistant (Claude Code, Aider, Copilot CLI) **faster, smarter, and more accurate.**

It provides a high-performance CLI that acts as an **O(1) Oracle** for your agent, allowing it to navigate massive codebases (1M+ lines) with zero hallucination and negligible token usage.

## Fast Track

### 1. Install
```bash
brew install rocket-tycoon/tap/rocketindex
```

### 2. Start RocketIndex
Run this in a separate terminal. It will index your codebase and watch for changes.

```bash
cd /path/to/your/repo
rkt watch
```

### 3. Augment Your Agent
Tell your agent (Claude Code, Aider, etc.) to use the tool:

> "I have installed `rkt`. Use `rkt symbols <query>` to find code and `rkt def <symbol>` to find definitions. Never use grep."

---

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

---

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

---

## Performance (Real-World Benchmarks)

Tested on **a huge Ruby on Rails monolith** (7,434 files).

| Query | RocketIndex | grep | Speedup |
| :--- | :--- | :--- | :--- |
| **Definition** | **0.010s** | 1.2s | **120x** |
| **References** | **0.020s** | 1.3s | **65x** |

*RocketIndex uses ~50MB RAM, constant. No V8 or JVM required.*

---

## Editor Integration
RocketIndex also powers the **F# Fast** extension for Zed. [See IDE Integration Guide](docs/ide_integration.md).

## Language Support
| Language | Extensions | Features |
| :--- | :--- | :--- |
| **F#** | `.fs`, `.fsi`, `.fsx` | Modules, functions, types, records, unions, interfaces |
| **Ruby** | `.rb` | Classes, modules, methods, constants |
| **Python** | `.py` | Classes, functions, methods |
| **JavaScript** | `.js`, `.jsx` | Classes, functions, variables, exports |
| **TypeScript** | `.ts`, `.tsx` | Classes, interfaces, types, functions, exports |
| **Rust** | `.rs` | Structs, enums, functions, traits, implementation blocks |
| **Go** | `.go` | Packages, functions, structs, interfaces, methods |

*(More coming via Tree-sitter)*

## License
[BSL 1.1](LICENSE) - Source Available.
