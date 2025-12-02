# fsharp-tools Feature Roadmap

**Created:** 2024-12-02
**Status:** Active

---

## Vision

fsharp-tools is a **fast, lightweight, predictable** F# language server. It prioritizes:
- Low memory usage (SQLite-backed, query on-demand)
- No runtime dependencies (pure Rust, no .NET required)
- AI/agent integration via CLI
- Instant startup and response times

It is **not** trying to replicate fsautocomplete. For full type-aware features, use fsautocomplete alongside fsharp-tools.

---

## Current State (Completed)

- [x] Go to definition (user code)
- [x] Find references
- [x] Workspace symbol search
- [x] Hover with type signatures (when extracted)
- [x] SQLite-backed index
- [x] F# compilation order support
- [x] CLI for scripting/AI integration
- [x] Type extraction via F# script

---

## Phase 1: Quick Wins ✅ COMPLETE

**Completed:** 2024-12-02
**Goal:** Polish the core experience with low-hanging fruit

### 1.1 Syntax Error Diagnostics ✅
- [x] Surface tree-sitter parse errors as LSP diagnostics
- [x] Show errors on file open, change, and save
- [x] Clear diagnostics when file is closed

### 1.2 Keyword Completion ✅
- [x] Complete 70+ F# keywords (let, match, type, module, etc.)
- [x] Filter by prefix as user types
- [x] Show "F# keyword" detail in completion menu

### 1.3 Symbol Completion ✅
- [x] Complete symbol names from current file
- [x] Complete symbol names from opened modules
- [x] Complete qualified names (Module.symbol)
- [x] Show symbol kind and qualified name in detail

### 1.4 Organize Opens ✅
- [x] Sort `open` statements alphabetically
- [x] Group by namespace depth (System before System.IO)
- [x] Available as "Organize imports" code action
- [x] Only offered when opens are out of order

---

## Phase 2: Enhanced Navigation

**Effort:** 2-4 weeks
**Goal:** Improve daily workflow with smarter features

### 2.1 Rename Symbol
- [ ] Rename across all files in workspace
- [ ] Preview changes before applying
- [ ] User code only (not external references)

### 2.2 Add Missing Open
- [ ] When symbol not found, search index
- [ ] Suggest `open ModuleName` as code action
- [ ] Quick fix from diagnostic

### 2.3 Member Completion
- [ ] Complete members after `.` using type cache
- [ ] Show method signatures in completion
- [ ] Requires type extraction to be run

### 2.4 External Symbol Support (Partial)
- [x] Index common .NET BCL types (System.*, System.IO.*, System.Linq.*, etc.)
- [x] Enable workspace symbol search for common types
- [ ] Read actual .NET assembly metadata (requires PE parsing)
- [ ] Index NuGet packages referenced in .fsproj
- [ ] Enable go-to-definition for external symbols (to metadata view)

**Note:** Current implementation provides ~80 commonly-used BCL types as a pragmatic
compromise. Full assembly reading is deferred to a future phase.

---

## Phase 3: Lightweight Inference

**Effort:** 2-3 months
**Goal:** Cover common cases without full type checking

### 3.1 Literal Type Inference
- [ ] `let x = 42` → infer `int`
- [ ] `let s = "hello"` → infer `string`
- [ ] `let b = true` → infer `bool`
- [ ] `let items = [1; 2; 3]` → infer `int list`

### 3.2 Constructor Type Inference
- [ ] `let u = User()` → infer `User`
- [ ] `let p = { Name = "x"; Age = 1 }` → infer record type

### 3.3 Annotation Propagation
- [ ] `let f (x: int) = x + 1` → infer `int -> int`
- [ ] Propagate through simple let bindings

### 3.4 Expression Type Display
- [ ] Show inferred types in hover
- [ ] ~60-70% coverage of common patterns

---

## Phase 4: Hybrid Mode (Optional)

**Effort:** Variable
**Goal:** Best of both worlds

### 4.1 fsautocomplete Integration
- [ ] Optional companion mode with fsautocomplete
- [ ] Use fsharp-tools for navigation (fast)
- [ ] Use fsautocomplete for diagnostics (accurate)
- [ ] Configuration to enable/disable

---

## Non-Goals

These features require the full F# compiler and are out of scope:

- Full type inference (Hindley-Milner + constraints)
- Overload resolution based on argument types
- Computation expression support
- Type provider support
- Refactorings that require type information
- Parity with fsautocomplete

---

## Feature Comparison

| Feature | fsharp-tools | fsautocomplete |
|---------|:------------:|:--------------:|
| Go to definition (user code) | ✅ | ✅ |
| Go to definition (external) | Partial* | ✅ |
| Find references | ✅ | ✅ |
| Workspace symbols | ✅ | ✅ |
| Hover (extracted types) | ✅ | ✅ |
| Hover (inferred types) | Phase 3 | ✅ |
| Syntax diagnostics | ✅ | ✅ |
| Type diagnostics | ❌ | ✅ |
| Keyword completion | ✅ | ✅ |
| Symbol completion | ✅ | ✅ |
| Type-aware completion | ❌ | ✅ |
| Organize opens | ✅ | ✅ |
| Rename symbol | ✅ | ✅ |
| Add missing open | ✅ | ✅ |
| Member completion | ✅** | ✅ |
| Other refactorings | ❌ | ✅ |
| Formatting | ❌ | ✅ |
| Memory usage | ~50 MB | 1-21 GB |
| Startup time | <100ms | 5-30s |
| .NET required | ❌ | ✅ |
| CLI for scripting | ✅ | ❌ |

*\* Partial: ~80 common BCL types indexed; full assembly reading not yet implemented*

*\*\* Requires type cache from build-time extraction*
