---
name: rocketindex
description: Code navigation and indexing (F#, Ruby)
triggers:
  - "definition"
  - "references"
  - "call graph"
  - "*.fs"
  - "*.fsproj"
  - "*.rb"
---

# RocketIndex - Code Navigation

When working with F# or Ruby codebases, use the `rkt` CLI:

## Commands

### Build Index
```bash
rkt index
```

### Find Definition
```bash
rkt def <qualified.name>
```

### Search Symbols
```bash
rkt symbols "Pattern*"
```

### Build Dependency Graph
```bash
rkt spider <Module.function> --depth 3
```

### Find Callers (Impact Analysis)
```bash
rkt callers <Symbol>
```

## When to Use
- Before modifying code, use `rkt callers` to understand impact
- Use `rkt def` instead of grep when looking for where something is defined
- Use `rkt symbols` for fuzzy symbol search
- Use `rkt spider` to explore dependencies

## Index Management
Build/rebuild: `rkt index`
The index is stored in SQLite at `.rocketindex/index.db` and survives session restarts.
