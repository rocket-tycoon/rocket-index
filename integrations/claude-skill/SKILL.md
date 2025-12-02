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
fsharp-index def <qualified.name>
```

### Find References
```bash
fsharp-index refs <file-path>
```

### Build Dependency Graph
```bash
fsharp-index spider <Module.function> --depth 3
```

## When to Use
- Before modifying F# code, spider from the function to understand impact
- Use `def` instead of grep when looking for where something is defined
- Use `refs` to find all usages before refactoring

## Index Management
Build/rebuild: `fsharp-index build --project <.fsproj>`
The index is stored in SQLite and survives session restarts.
