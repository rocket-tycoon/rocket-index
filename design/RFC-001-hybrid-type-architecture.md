# RFC-001: Hybrid Type Architecture

**Status:** Draft
**Created:** 2024-12-02
**Author:** fsharp-tools team

## Summary

Extend fsharp-tools with type-aware symbol resolution by extracting type information at build time, while keeping the runtime tool pure Rust with bounded memory. A future phase adds lightweight type inference in Rust for real-time analysis of simple cases.

## Motivation

### Current Limitation

fsharp-tools uses syntactic analysis only (tree-sitter). This means we cannot resolve:

```fsharp
// Member access on inferred types
let len = myString.Length        // Need to know myString: string
let items = response.Items       // Need to know response type

// Implicit type-directed resolution
let result = xs |> map ((+) 1)   // 'map' without 'List.' prefix
```

### Why Not Just Use FCS at Runtime?

FSharp.Compiler.Service (FCS) / fsautocomplete has:
- Unbounded memory growth (1GB+ observed)
- Slow startup (2-3s to load project)
- Heavy resource usage when resident

These problems motivated fsharp-tools in the first place. Reintroducing FCS at runtime—even with timeouts—risks bringing back these issues.

### The Insight

Users already run `dotnet build`, which invokes FCS. We can **extract type information once at build time** and store it for the Rust tool to read. This gives us:

- Type-aware resolution when available
- Zero runtime .NET dependency
- Bounded, predictable memory (<50MB)
- Fast queries (no FCS startup cost)

## Design

### Phase 1: Build-Time Type Extraction

#### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Build Phase (one-time)                  │
│                                                             │
│   dotnet build ──► Post-build hook ──► .fsharp-types/       │
│                    (F# script)          cache.json          │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Runtime (Rust, bounded)                  │
│                                                             │
│   fsharp-index ◄── merges ◄── .fsharp-index/ (syntactic)   │
│        │                       .fsharp-types/ (semantic)    │
│        ▼                                                    │
│   Type-aware go-to-definition, symbol search, spider        │
└─────────────────────────────────────────────────────────────┘
```

#### Type Cache Format

```json
{
  "version": 1,
  "extracted_at": "2024-12-02T10:30:00Z",
  "project": "RocketSpec.Core",
  "symbols": [
    {
      "name": "myString",
      "qualified": "MyModule.myString",
      "type": "string",
      "file": "src/MyModule.fs",
      "line": 42
    },
    {
      "name": "processUser",
      "qualified": "UserService.processUser",
      "type": "User -> Async<Result<Response, Error>>",
      "file": "src/UserService.fs",
      "line": 15,
      "parameters": [
        { "name": "user", "type": "User" }
      ]
    }
  ],
  "members": [
    {
      "type": "User",
      "member": "Name",
      "member_type": "string",
      "kind": "property"
    },
    {
      "type": "User",
      "member": "Save",
      "member_type": "unit -> Async<unit>",
      "kind": "method"
    }
  ]
}
```

#### Extraction Script

A small F# script (~100 lines) using FSharp.Compiler.Service:

1. Load the .fsproj
2. Run type checking (FCS does this during build anyway)
3. Walk the typed AST
4. Extract symbol names, types, and member signatures
5. Write to `.fsharp-types/cache.json`

This runs **once per build**, not continuously.

#### Integration Points

**Option A: MSBuild Target**
```xml
<Target Name="ExtractFSharpTypes" AfterTargets="Build">
  <Exec Command="dotnet fsi extract-types.fsx $(ProjectPath)" />
</Target>
```

**Option B: Manual / CI Hook**
```bash
dotnet build && dotnet fsi extract-types.fsx MyProject.fsproj
```

**Option C: fsharp-index CLI**
```bash
fsharp-index build --extract-types  # runs extraction after indexing
```

#### Rust Integration

```rust
// In fsharp-index

pub struct TypeCache {
    symbols: HashMap<String, TypeInfo>,
    members: HashMap<String, Vec<MemberInfo>>,
}

impl TypeCache {
    pub fn load(path: &Path) -> Result<Self> { ... }

    pub fn get_type(&self, qualified_name: &str) -> Option<&str> { ... }

    pub fn get_members(&self, type_name: &str) -> Option<&[MemberInfo]> { ... }
}

// Enhanced resolution
pub fn resolve_member_access(&self, expr: &str, member: &str) -> Option<Symbol> {
    // 1. Find the expression's type from cache
    // 2. Look up member on that type
    // 3. Return definition location
}
```

### Phase 2: Lightweight Rust Inference (Future)

For real-time analysis without waiting for a build, implement simple type propagation in Rust:

#### Supported Cases

```fsharp
// Explicit annotations - extract type from syntax
let x: string = getValue()
x.Length  // ✅ Resolve via type cache

// Obvious literals
let items = [1; 2; 3]      // int list
let name = "hello"          // string
let flag = true             // bool

// Constructor calls
let user = User.Create()    // User (if User is in index)
user.Name                   // ✅ Resolve via type cache

// Function return types (from signatures in index)
let parse: string -> int option = ...
let result = parse "42"     // int option
result.Value               // ✅ Resolve via type cache
```

#### Not Supported (Requires Full Inference)

```fsharp
// Complex generic inference
let xs = items |> List.map (fun x -> x.ToString())
// Type of xs depends on items, which depends on...

// Implicit operator resolution
let sum = a + b  // Which (+) operator?

// SRTP / inline constraints
let inline add x y = x + y
```

#### Implementation Sketch

```rust
pub enum InferredType {
    Known(String),           // "string", "int", "User"
    List(Box<InferredType>), // [1;2;3] -> List(Known("int"))
    Option(Box<InferredType>),
    Unknown,
}

pub fn infer_binding_type(node: &SyntaxNode) -> InferredType {
    // Check for explicit annotation
    if let Some(annotation) = find_type_annotation(node) {
        return InferredType::Known(annotation);
    }

    // Check for literal
    if let Some(literal_type) = infer_literal(node) {
        return literal_type;
    }

    // Check for constructor call
    if let Some(ctor) = find_constructor_call(node) {
        return InferredType::Known(ctor.type_name);
    }

    InferredType::Unknown
}
```

## Tradeoffs

### Phase 1: Build-Time Extraction

| Aspect | Assessment |
|--------|------------|
| **Memory** | ✅ Bounded - cache is static JSON |
| **Speed** | ✅ Fast - just JSON parsing |
| **Accuracy** | ✅ Perfect - from compiler |
| **Freshness** | 🟡 Stale between builds |
| **Setup** | 🟡 Requires build hook |

**Staleness is acceptable because:**
- Navigation targets don't change often
- Users rebuild when editing
- Syntactic index handles new symbols immediately
- Type cache enhances, doesn't replace

### Phase 2: Rust Inference

| Aspect | Assessment |
|--------|------------|
| **Memory** | ✅ Bounded - pure computation |
| **Speed** | ✅ Fast - local analysis |
| **Accuracy** | 🟡 Partial - simple cases only |
| **Freshness** | ✅ Real-time |
| **Complexity** | 🔴 Significant implementation effort |

## Alternatives Considered

### A. FCS at Runtime (Rejected)

Spawn FSharp.Compiler.Service on-demand, kill after timeout.

**Rejected because:**
- FCS takes 2-3s to initialize
- Memory usage during query (200-500MB)
- Designed for resident operation, not ephemeral
- Risk of reintroducing the problems we're solving

### B. Parse DLL Metadata (Deferred)

Read type information directly from compiled assemblies.

**Deferred because:**
- Complex (.NET metadata tables)
- Build-time extraction is simpler and more accurate
- Could add later for NuGet packages without source

### C. Ship FSharp.Core Stubs (Complementary)

Hand-written .fsi files for common libraries.

**Status:** Complementary to this RFC. Can ship stubs for FSharp.Core regardless of this design.

## Implementation Plan

### Phase 1 Milestones

1. **Design type cache schema** (this RFC)
2. **Write extraction script** (~100 lines F#)
3. **Add TypeCache to fsharp-index** (Rust)
4. **Integrate with resolution** (enhance `resolve.rs`)
5. **Add CLI integration** (`fsharp-index build --extract-types`)
6. **Documentation and testing**

### Phase 2 Milestones (Future)

1. **Design inference rules**
2. **Implement literal inference**
3. **Implement annotation extraction**
4. **Implement constructor tracking**
5. **Integration and fallback logic**

## Open Questions

1. **Cache invalidation:** Should we store a hash of source files to detect staleness?

2. **Multi-project solutions:** Extract per-project or solution-wide?

3. **Incremental extraction:** Can we update cache for just changed files?

4. **External dependencies:** Should extraction include NuGet package types?

## References

- [FSharp.Compiler.Service](https://fsharp.github.io/fsharp-compiler-docs/)
- [fsautocomplete](https://github.com/fsharp/FsAutoComplete)
- [tree-sitter-fsharp](https://github.com/ionide/tree-sitter-fsharp)
