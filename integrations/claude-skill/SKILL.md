---
name: rocketindex
description: F# code navigation and indexing
triggers:
  - "F# definition"
  - "F# references"
  - "F# call graph"
  - "*.fs"
  - "*.fsproj"
---

# RocketIndex - F# Code Navigation

When working with F# codebases, use the `rocketindex` CLI:

## Commands

### Build Index
```bash
rocketindex build
```

### Find Definition
```bash
rocketindex def <qualified.name>
```

### Search Symbols
```bash
rocketindex symbols "Pattern*"
```

### Build Dependency Graph
```bash
rocketindex spider <Module.function> --depth 3
```

## When to Use
- Before modifying F# code, spider from the function to understand impact
- Use `def` instead of grep when looking for where something is defined
- Use `symbols` for fuzzy symbol search

## Index Management
Build/rebuild: `rocketindex build`
The index is stored in SQLite at `.rocketindex/index.db` and survives session restarts.
