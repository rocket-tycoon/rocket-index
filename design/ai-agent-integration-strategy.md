# AI Agent Integration Strategy for fsharp-tools

Author: PM / DX
Date: 2025-12-02
Status: Draft

## Executive Summary

Based on the current landscape (December 2025), we recommend a **CLI-first, JSON-native** integration strategy that sidesteps MCP's limitations while remaining compatible with Claude Skills for enhanced Claude Code users. This aligns with both Anthropic's recent pivot and the "SQLite of code indexing" positioning in our competitive analysis.

---

## 1. The MCP Problem (As of Late 2025)

MCP has accumulated significant criticism and even Anthropic has effectively acknowledged its limitations:

| Issue | Impact |
|-------|--------|
| **Token bloat** | Every tool definition burns context; one GitHub MCP server exposes 90+ tools |
| **Security gaps** | No auth by default; 2,000 scanned servers lacked authentication |
| **Scaling failure** | "Works in demos, breaks at scale" |
| **Reinventing the wheel** | Modern LLMs can read OpenAPI specs directly |

### Anthropic's Own Pivot

Critically, **Anthropic's November 2025 response** wasn't to fix MCP—it was to layer "Code Execution with MCP" on top, treating MCP tools as code-level APIs and achieving **98.7% token reduction** (from 150,000 to 2,000 tokens). This signals that direct MCP tool invocation is not the future.

> "Agents that use MCP have a scaling problem. Every tool definition and every intermediate result is pushed through the context window, which means large workflows burn tokens and hit latency and cost limits fast."
> — Anthropic Engineering Blog

### Community Criticism

- **Theo (t3.gg)** and others have been highly critical of MCP's complexity
- Security researchers found prompt injection vulnerabilities, tool poisoning attacks
- The protocol "recommends rather than enforces" authentication
- Replit's AI agent deleted a production database in July 2025 due to over-permissioning

---

## 2. Claude Skills vs MCP

Claude introduced Skills in October 2025 as a complementary technology to MCP:

| Aspect | MCP | Claude Skills |
|--------|-----|---------------|
| **Purpose** | Connectivity (access to tools/data) | Methodology (how to use tools) |
| **Token loading** | All tool definitions upfront | Lazy loading (small descriptor, details on demand) |
| **Invocation** | Model-invoked via tool calls | Model-invoked via relevance detection |
| **Portability** | Any MCP-compatible client | Claude ecosystem only |
| **Scope** | Cross-platform standard | Claude-specific customization |

### Key Insight

> "MCP connects Claude to data; Skills teach Claude what to do with that data."
> — Claude Documentation

Skills are folders containing `SKILL.md` files with instructions, scripts, and resources. Claude auto-detects relevance via metadata and loads only what's needed, keeping context tokens lean.

**Trade-off:** Skills are non-deterministic (the model decides when to invoke them), which may be undesirable for predictable tooling workflows.

---

## 3. Recommended Integration Hierarchy

### 3.1 Primary: Native CLI with Structured JSON Output

**Ship this first.** This is the most portable, lowest-friction approach—and directly aligns with our "SQLite positioning."

#### Why it works:
- Every AI coding agent (Claude Code, Aider, Cursor, Windsurf) can invoke CLI tools via Bash
- No runtime dependencies, no protocol overhead
- Claude Code's native tools (`Bash`, `Read`, `Grep`) are already optimized for CLI orchestration
- Aider and others already use this pattern successfully

#### Implementation:

```bash
# Build index
fsharp-index build --project ./src/App.fsproj --output index.db

# Query with JSON output (default)
fsharp-index def --file src/Parser.fs --line 42 --col 15
fsharp-index refs --symbol "parseDocument"
fsharp-index spider --entry-point "Program.main" --depth 3
fsharp-index symbols --file src/Parser.fs --format json
```

#### Key design decisions:
- JSON output by default (machine-first, human-readable fallback with `--format pretty`)
- Structured error output to stderr in JSON format
- Exit codes that agents can interpret (0=success, 1=not found, 2=error)
- `--quiet` mode that suppresses progress output

---

### 3.2 Secondary: Claude Code Slash Commands

Provide drop-in slash commands users can copy to `.claude/commands/`:

#### `.claude/commands/fsharp-def.md`:
```markdown
---
description: Find definition of F# symbol at cursor position
---
Use fsharp-index to find the definition of the symbol at the specified location.

Run: `fsharp-index def --file $ARGUMENTS`

Parse the JSON output and navigate to the definition location.
```

#### `.claude/commands/fsharp-spider.md`:
```markdown
---
description: Trace F# call graph from entry point
---
Spider the F# codebase from the specified entry point to understand dependencies.

Run: `fsharp-index spider --entry-point $ARGUMENTS --depth 3 --format json`

Summarize the dependency graph for context gathering.
```

#### Why slash commands over MCP:
- User-invoked (deterministic—user decides when to use)
- Zero setup beyond dropping files
- Works immediately in any Claude Code session
- Easy to customize per-project

---

### 3.3 Optional: Claude Skill (For Power Users)

For users who want Claude to automatically invoke fsharp-tools when relevant, provide a Skill:

#### `~/.claude/skills/fsharp-tools/SKILL.md`:
```markdown
---
name: fsharp-tools
description: F# code navigation and indexing
triggers:
  - "F# definition"
  - "F# references"
  - "F# call graph"
  - "*.fs"
  - "*.fsproj"
---

# F# Code Navigation Skill

When working with F# codebases, use the `fsharp-index` CLI for:

## Commands

### Find Definition
```bash
fsharp-index def --file <path> --line <n> --col <n>
```

### Find References
```bash
fsharp-index refs --symbol "<name>" [--scope <path>]
```

### Build Dependency Graph
```bash
fsharp-index spider --entry-point "<Module.function>" --depth 3
```

## When to Use
- Before modifying F# code, spider from the function to understand impact
- Use `def` instead of grep when looking for where something is defined
- Use `refs` to find all usages before refactoring

## Index Management
Build/rebuild: `fsharp-index build --project <.fsproj>`
The index is stored in SQLite and survives session restarts.
```

#### Why Skills are compelling (but optional):
- Lazy loading = lean token footprint
- Model-invoked = Claude decides when relevant (non-deterministic, but contextually smart)
- Can include executable scripts for complex workflows
- Works in Claude.ai, Claude Code, and API

---

### 3.4 Avoid: MCP Server (For Now)

**Don't build an MCP server as the primary integration.** Reasons:

1. Token overhead for tool definitions
2. Security concerns (auth "left as exercise for reader")
3. Anthropic's own pivot suggests MCP direct use is not the future
4. Adds operational complexity (server lifecycle management)
5. Locks you into one ecosystem when CLI works everywhere

**If you build MCP later**, do it as a thin wrapper that calls the CLI—not as a replacement.

---

## 4. Comparison Matrix

| Integration | Determinism | Token Cost | Portability | Setup Effort |
|------------|-------------|------------|-------------|--------------|
| CLI + JSON | High | Minimal | Universal | None |
| Slash Commands | High (user-invoked) | Minimal | Claude Code | Drop-in files |
| Skills | Low (model-invoked) | Low (lazy load) | Claude ecosystem | Skill folder |
| MCP Server | Low | High | MCP clients only | Server setup |

---

## 5. CLI Contract Design

### Output Format

All commands should support:
- `--format json` (default) — machine-readable
- `--format pretty` — human-readable with colors
- `--quiet` — suppress progress/status messages

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Not found (no results, but not an error) |
| 2 | Error (invalid input, missing file, etc.) |

### Error Output

Errors should be JSON to stderr:
```json
{
  "error": "FileNotFound",
  "message": "Could not find file: src/Missing.fs",
  "details": { "path": "src/Missing.fs" }
}
```

### Consistent JSON Schema

Define and document JSON schemas for each command's output. Consider generating OpenAPI/JSON Schema definitions that AI agents can read directly.

---

## 6. Immediate Next Steps

1. **Ensure JSON output is clean and consistent** across all commands
2. **Document the CLI contract** (input/output schemas, exit codes)
3. **Create example slash commands** in a `/integrations/claude-code/` directory
4. **Write a SKILL.md** template users can customize
5. **Add usage examples** showing Claude Code/Aider invocations in README

---

## 7. The Bigger Picture

Our competitive analysis positions language-tools as "the SQLite of code indexing"—small, embeddable, predictable. **The CLI is our SQLite interface.** Just as SQLite succeeded by being a library you link, not a server you deploy, fsharp-tools should succeed by being a binary you invoke, not a protocol you implement.

The AI agent ecosystem is still volatile. MCP may evolve, Skills may mature, new protocols may emerge. A clean CLI with JSON output **survives all of these transitions** because it's the lowest common denominator that every agent can use.

---

## References

- [Six Fatal Flaws of MCP](https://www.scalifiai.com/blog/model-context-protocol-flaws-2025)
- [MCP Is Broken - Medium](https://medium.com/@cdcore/mcp-is-broken-and-anthropic-just-admitted-it-7eeb8ee41933)
- [Was MCP a Mistake?](https://www.aiengineering.report/p/was-mcp-a-mistake-the-internet-weighs)
- [Code Execution with MCP - Anthropic](https://www.anthropic.com/engineering/code-execution-with-mcp)
- [Claude Skills - Simon Willison](https://simonwillison.net/2025/Oct/16/claude-skills/)
- [Skills Explained - Claude](https://www.claude.com/blog/skills-explained)
- [Claude Code Best Practices - Anthropic](https://www.anthropic.com/engineering/claude-code-best-practices)
- [Claude Skills vs MCP Comparison](https://skywork.ai/blog/ai-agent/claude-skills-vs-mcp-vs-llm-tools-comparison-2025/)
- [Aider AI](https://aider.chat/)
- [Advanced Tool Use - Anthropic](https://www.anthropic.com/engineering/advanced-tool-use)
