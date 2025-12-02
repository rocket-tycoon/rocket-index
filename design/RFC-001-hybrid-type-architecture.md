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

### Storage Format: SQLite

We use SQLite instead of JSON for the index storage. This provides:

| Benefit | Description |
|---------|-------------|
| **Indexed lookups** | O(log n) queries by qualified name, no full scan |
| **Low memory** | Query on-demand, don't load entire index into RAM |
| **Incremental updates** | INSERT/UPDATE single rows, no full rewrite |
| **Rich queries** | `LIKE` patterns, JOINs for references |
| **Debuggable** | Inspect with `sqlite3` CLI or DB Browser |
| **Battle-tested** | Proven in countless tools |

This applies to both the syntactic index (`.fsharp-index/index.db`) and the type cache (`.fsharp-index/types.db`), which can be merged into a single database file.

### Phase 1: Build-Time Type Extraction

#### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Build Phase (one-time)                  │
│                                                             │
│   dotnet build ──► Post-build hook ──► .fsharp-index/       │
│                    (F# script)          index.db            │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Runtime (Rust, bounded)                  │
│                                                             │
│   fsharp-index ◄── queries ◄── .fsharp-index/index.db      │
│        │                       (syntactic + semantic)       │
│        ▼                                                    │
│   Type-aware go-to-definition, symbol search, spider        │
└─────────────────────────────────────────────────────────────┘
```

#### Database Schema

```sql
-- Metadata table for versioning and staleness detection
CREATE TABLE metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- e.g., ("schema_version", "1"), ("extracted_at", "2024-12-02T10:30:00Z")

-- Main symbols table (syntactic + type info merged)
CREATE TABLE symbols (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    qualified TEXT NOT NULL,
    kind TEXT NOT NULL,          -- 'function', 'value', 'type', 'module', etc.
    type_signature TEXT,         -- NULL for syntactic-only, populated by FCS extraction
    file TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER NOT NULL,
    end_line INTEGER,
    end_column INTEGER,
    visibility TEXT DEFAULT 'public',
    source TEXT DEFAULT 'syntactic'  -- 'syntactic' or 'semantic'
);

CREATE INDEX idx_symbols_qualified ON symbols(qualified);
CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_symbols_file ON symbols(file);
CREATE INDEX idx_symbols_kind ON symbols(kind);

-- Type members (properties, methods) for member access resolution
CREATE TABLE members (
    id INTEGER PRIMARY KEY,
    type_name TEXT NOT NULL,      -- Fully qualified type name
    member_name TEXT NOT NULL,
    member_type TEXT,             -- Type signature
    kind TEXT NOT NULL,           -- 'property', 'method', 'field'
    file TEXT,                    -- Definition location (if known)
    line INTEGER
);

CREATE INDEX idx_members_type ON members(type_name);
CREATE INDEX idx_members_name ON members(member_name);

-- References for find-usages (optional, populated by syntactic pass)
CREATE TABLE references (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    file TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER NOT NULL
);

CREATE INDEX idx_references_name ON references(name);
CREATE INDEX idx_references_file ON references(file);

-- Open statements for resolution context
CREATE TABLE opens (
    id INTEGER PRIMARY KEY,
    file TEXT NOT NULL,
    module_path TEXT NOT NULL,
    line INTEGER NOT NULL
);

CREATE INDEX idx_opens_file ON opens(file);
```

#### Extraction Script

A small F# script (~100 lines) using FSharp.Compiler.Service:

1. Load the .fsproj
2. Run type checking (FCS does this during build anyway)
3. Walk the typed AST
4. Extract symbol names, types, and member signatures
5. INSERT/UPDATE into `.fsharp-index/index.db`

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
use rusqlite::{Connection, params};

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }

    pub fn find_symbol(&self, qualified: &str) -> Result<Option<Symbol>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT name, qualified, kind, type_signature, file, line, column
             FROM symbols WHERE qualified = ?"
        )?;
        // ... map to Symbol struct
    }

    pub fn search_symbols(&self, pattern: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT name, qualified, kind, type_signature, file, line, column
             FROM symbols WHERE name LIKE ? OR qualified LIKE ?
             LIMIT 100"
        )?;
        // ... map to Vec<Symbol>
    }

    pub fn get_members(&self, type_name: &str) -> Result<Vec<Member>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT member_name, member_type, kind, file, line
             FROM members WHERE type_name = ?"
        )?;
        // ... map to Vec<Member>
    }

    pub fn resolve_member_access(&self, expr_type: &str, member: &str) -> Result<Option<Location>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT file, line FROM members
             WHERE type_name = ? AND member_name = ?"
        )?;
        // ... return location
    }
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

pub fn infer_binding_type(node: &SyntaxNode, index: &Index) -> InferredType {
    // Check for explicit annotation
    if let Some(annotation) = find_type_annotation(node) {
        return InferredType::Known(annotation);
    }

    // Check for literal
    if let Some(literal_type) = infer_literal(node) {
        return literal_type;
    }

    // Check for constructor call - look up in index
    if let Some(ctor) = find_constructor_call(node) {
        if index.find_symbol(&ctor.type_name).is_ok() {
            return InferredType::Known(ctor.type_name);
        }
    }

    InferredType::Unknown
}
```

## Tradeoffs

### Phase 1: Build-Time Extraction with SQLite

| Aspect | Assessment |
|--------|------------|
| **Memory** | ✅ Bounded - query on-demand, don't load all |
| **Speed** | ✅ Fast - indexed O(log n) lookups |
| **Accuracy** | ✅ Perfect - from compiler |
| **Freshness** | 🟡 Stale between builds |
| **Setup** | 🟡 Requires build hook |
| **Incremental** | ✅ UPDATE single rows |

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

### B. JSON Storage (Rejected)

Store index as JSON files.

**Rejected because:**
- Must load entire file into memory to query
- O(n) lookups unless HashMap built after parse
- Full rewrite needed for any update
- Parse time grows with file size

SQLite provides indexed lookups, on-demand queries, and incremental updates.

### C. Parse DLL Metadata (Deferred)

Read type information directly from compiled assemblies.

**Deferred because:**
- Complex (.NET metadata tables)
- Build-time extraction is simpler and more accurate
- Could add later for NuGet packages without source

### D. Ship FSharp.Core Stubs (Complementary)

Hand-written .fsi files for common libraries.

**Status:** Complementary to this RFC. Can ship stubs for FSharp.Core regardless of this design.

## Implementation Plan

### Phase 1 Milestones

1. **Migrate syntactic index to SQLite** - Replace JSON with SQLite for existing functionality
2. **Design and create schema** - Tables for symbols, members, references, opens
3. **Write extraction script** (~100 lines F#) - Populate type_signature and members
4. **Update Rust queries** - Use rusqlite for all index operations
5. **Add CLI integration** (`fsharp-index build --extract-types`)
6. **Documentation and testing**

### Phase 2 Milestones (Future)

1. **Design inference rules**
2. **Implement literal inference**
3. **Implement annotation extraction**
4. **Implement constructor tracking**
5. **Integration and fallback logic**

## Migration Path

The migration from JSON to SQLite can be done incrementally:

1. Add rusqlite dependency
2. Create schema and write path
3. Dual-write: JSON + SQLite during transition
4. Verify SQLite queries match JSON behavior
5. Switch reads to SQLite
6. Remove JSON code path

Existing `.fsharp-index/index.json` files will be automatically migrated on first `fsharp-index build`.

## Open Questions

1. **Single vs multiple DB files:** One `index.db` or separate `syntactic.db` + `types.db`?
   - **Recommendation:** Single file for simplicity, separate tables for concerns

2. **Cache invalidation:** Store file content hashes to detect staleness?
   - **Recommendation:** Yes, add `file_hash` column to detect stale type info

3. **Multi-project solutions:** Per-project DB or solution-wide?
   - **Recommendation:** Per-project, with cross-project resolution via qualified names

4. **External dependencies:** Include NuGet package types?
   - **Recommendation:** Defer to Phase 2 or separate RFC

## References

- [FSharp.Compiler.Service](https://fsharp.github.io/fsharp-compiler-docs/)
- [fsautocomplete](https://github.com/fsharp/FsAutoComplete)
- [tree-sitter-fsharp](https://github.com/ionide/tree-sitter-fsharp)
- [rusqlite](https://github.com/rusqlite/rusqlite)
