# AGENTS.md vs MCP Server Spike Results

**Date**: 2025-12-09
**Branch**: spike/agents-md-vs-mcp

## Hypothesis

MCP server will show more consistent rkt tool usage because tools appear in the
agent's tool list, while AGENTS.md requires the agent to read and follow markdown
instructions.

## Methodology

Task: "Find all functions that call ApiProviderFactory. List each caller with file path and function name."

This task specifically tests `find_callers` - a capability where rkt provides unique value
(semantic caller identification vs grep text matching).

Tests run in vets-api (large Ruby Rails app: 7,470 files, 37K symbols).

## Results

| Model | Mode | Turns | Status | Cost |
|-------|------|-------|--------|------|
| Haiku | MCP enabled | 2 | SUCCESS (16 callers found) | ~$0.03 |
| Haiku | MCP disabled (AGENTS.md only) | 10 | FAILED (max_turns) | - |
| Sonnet | MCP enabled | 2 | SUCCESS | $0.12 |
| Sonnet | MCP disabled (AGENTS.md only) | 10 | FAILED (max_turns) | $0.15 |

## Analysis

### Key Findings

1. **MCP provides 5x turn reduction and 100% success rate**
   - With MCP: 2 turns to complete
   - Without MCP: 10 turns (max) and still failed

2. **AGENTS.md instructions were NOT followed**
   - Despite `.rocketindex/AGENTS.md` containing explicit instructions to use `rkt callers`
   - The agent defaulted to grep-based search patterns
   - This confirms the hypothesis: AGENTS.md is "hope-based" instruction following

3. **Tool visibility matters more than documentation**
   - When `find_callers` appears in the tool list, the agent uses it immediately
   - When only CLI instructions exist, the agent falls back to familiar patterns (grep)

### Why AGENTS.md Failed

The vets-api codebase has an AGENTS.md file with:
```
| Instead of | Use |
|------------|-----|
| `grep -r "functionName"` to find usages | `rkt callers "functionName"` |
```

But without MCP:
1. Agent didn't read AGENTS.md (or didn't prioritize it)
2. Agent used default grep/ripgrep search patterns
3. Large codebase = many results = lots of turns to filter
4. Never achieved the "semantic caller" precision that rkt provides

## Conclusion

**AGENTS.md CLI instructions are NOT a viable replacement for MCP server tools.**

The MCP server provides:
- **Tool visibility**: `find_callers` appears as an option, not buried in markdown
- **Deterministic selection**: Agent sees tool description that matches the query
- **Structured output**: No need to parse CLI output
- **Reliability**: 100% success vs 0% in this test

## Recommendation

Keep the MCP server. AGENTS.md is useful as:
- Fallback documentation for non-MCP agents
- Human reference for CLI commands
- Watch mode instructions (can't be an MCP tool)

But for consistent, reliable tool usage, MCP is essential.
