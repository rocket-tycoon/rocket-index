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

When working with F# codebases, use the `rkt` CLI:

## Commands

### Build Index
```bash
rktbuild
```

### Find Definition
```bash
rktdef <qualified.name>
```

### Search Symbols
```bash
rktsymbols "Pattern*"
```

### Build Dependency Graph
```bash
rktspider <Module.function> --depth 3
```

## When to Use
- Before modifying F# code, spider from the function to understand impact
- Use `def` instead of grep when looking for where something is defined
- Use `symbols` for fuzzy symbol search

## Index Management
Build/rebuild: `rktbuild`
The index is stored in SQLite at `.rocketindex/index.db` and survives session restarts.
