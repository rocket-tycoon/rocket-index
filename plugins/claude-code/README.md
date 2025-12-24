# RocketIndex Plugin for Claude Code

Semantic code navigation that "just works" - find callers, definitions,
and dependency graphs without grep or manual configuration.

## Features

- **find_callers**: Find all functions that call a symbol
- **find_definition**: Locate where a symbol is defined
- **find_references**: Find all references to a symbol
- **analyze_dependencies**: Traverse call graphs (forward or reverse)
- **search_symbols**: Pattern search with wildcards and fuzzy matching
- **describe_project**: Get a semantic map of project structure

Supports 12 languages: C, C++, C#, F#, Go, Java, JavaScript, PHP, Python, Ruby, Rust, TypeScript

## Installation

### Prerequisites

RocketIndex CLI must be installed and available as `rkt` in your PATH:

```bash
# macOS (Homebrew)
brew install rocket-tycoon/tap/rocketindex

# From source
cargo install --path crates/rocketindex-cli
```

### Install Plugin

```bash
# From local path
claude plugins add /path/to/RocketIndex/plugins/claude-code

# From GitHub (when published)
claude plugins add github:rocket-tycoon/rocket-index/plugins/claude-code
```

## How It Works

1. **MCP Server**: The plugin automatically starts `rkt serve` as an MCP server
2. **Project Discovery**: A PreToolUse hook automatically injects Claude's working
   directory as `project_root` for all RocketIndex tools
3. **JIT Indexing**: When you first use a tool in a new project, RocketIndex
   automatically creates the index

No manual `rkt serve add` or `rkt index` commands needed!

## Usage

Just ask Claude Code to use semantic navigation:

```
"Who calls the processPayment function?"
"Find the definition of UserService"
"Show me the dependency graph starting from main()"
```

Claude will automatically use the RocketIndex tools instead of grep when appropriate.

## Overriding Project Root

If you need to query a different project, specify `project_root` explicitly:

```
"Find callers of processPayment in /path/to/other/project"
```

The hook respects explicitly-specified project roots and won't override them.

## Troubleshooting

### "rkt: command not found"

Ensure RocketIndex is installed and in your PATH:

```bash
which rkt
# Should output something like /opt/homebrew/bin/rkt
```

### Index not updating

RocketIndex watches files automatically. If updates aren't reflected:

```bash
# Force reindex
rkt index
```

## Uninstall

```bash
claude plugins remove rocketindex
```
