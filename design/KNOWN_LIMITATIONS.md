# Known Limitations

This document describes the architectural limitations of fsharp-tools. These are deliberate trade-offs made to achieve the project's goals of minimal resource usage, fast performance, and no .NET runtime dependency.

---

## 1. No Type-Aware Analysis

**Limitation:** fsharp-tools uses tree-sitter for syntactic analysis only. It has no understanding of F# types, type inference, or the F# type system.

**Impact:**
- Cannot resolve overloaded methods based on argument types
- Cannot distinguish between identically-named symbols with different type signatures
- No type information in hover or completion
- Cannot detect type errors

**Why:** Type-aware analysis requires the F# compiler or a reimplementation of F#'s complex type inference. This would require either the .NET runtime or massive implementation effort—both against project goals.

**Workaround:** For type-aware features, use the official F# language server (fsautocomplete) alongside fsharp-tools. Optional type extraction (via `scripts/extract-types.fsx` or `fsharp-index build --extract-types`) can stash known signatures into the SQLite index for richer hover/member hints, but it still cannot infer pipeline types or disambiguate overloads automatically.

---

## 2. F# Compilation Order ✅ Now Supported

**Status:** As of the latest update, fsharp-tools now parses `.fsproj` files to extract compilation order.

**How it works:**
- During `fsharp-index build`, all `.fsproj` files in the workspace are parsed
- `<Compile Include="..."/>` elements are extracted in order
- Name resolution respects this order: symbols are only visible if their defining file comes before the referencing file
- Spider dependency graphs respect compilation order

**Remaining limitations:**
- Multi-project solutions: project references are parsed but cross-project visibility is approximate
- If no `.fsproj` is found, falls back to permissive mode (all files can reference all files)

---

## 3. O(N) Workspace Symbol Search

**Limitation:** Workspace symbol search performs a linear scan over all indexed symbols, with string allocations on each query.

**Impact:**
- Search may be slow for very large codebases (>10k symbols)
- Increased GC pressure during interactive use

**Why:** For typical F# project sizes, linear search is fast enough. Adding a prefix trie or ngram index would increase complexity and memory usage.

**Workaround:** Keep search queries specific to reduce result sets.

---

## 4. Approximate Spider Dependency Graph

**Limitation:** The spider's dependency graph is built using best-effort name resolution. When a symbol cannot be resolved precisely, it falls back to the first search result.

**Impact:**
- Dependency edges may be incorrect
- External dependencies (FSharp.Core, NuGet packages) appear as "unresolved"
- May include spurious dependencies from ambiguous names

**Why:** Precise dependency resolution requires type information and access to compiled assemblies.

**Workaround:** Treat spider output as an approximation. Review "unresolved" references for false positives.

---

## 5. No Understanding of Type Inference

**Limitation:** F# relies heavily on type inference. fsharp-tools cannot infer types and therefore cannot resolve method calls on inferred types.

**Example:**
```fsharp
let items = [1; 2; 3]  // type inferred as int list
items |> List.map (fun x -> x + 1)  // we can't resolve this
```

**Impact:**
- Cannot resolve method calls on values with inferred types
- Cannot navigate to correct overload
- Pipeline operators (`|>`, `>>`) may not resolve correctly

**Workaround:** The optional type extraction pipeline can capture known signatures for hover display, but it does not perform inference. Use fsautocomplete when you need live type reasoning.

---

## 6. Limited Overload Resolution

**Limitation:** When multiple symbols have the same qualified name (overloaded methods), fsharp-tools returns the most recently indexed one.

**Impact:**
- Go-to-definition may jump to wrong overload
- "Get all overloads" is available but caller must choose manually

**Why:** Correct overload resolution requires type information about call-site arguments.

---

## 7. No External Dependency Resolution

**Limitation:** Symbols from NuGet packages, FSharp.Core, and the .NET BCL are not indexed.

**Impact:**
- Cannot navigate to external definitions
- External symbols appear as "unresolved" in spider
- No hover information for external symbols

**Why:** Indexing external dependencies would require downloading packages, extracting metadata from compiled assemblies, and potentially running .NET.

**Workaround:** External dependencies are marked as `<external>` in spider output. Use IDE features for navigation to external code.

---

## 8. Module Aliases Not Tracked

**Limitation:** F# allows module aliases (`module L = List`). These are not currently tracked or resolved.

**Impact:**
- Symbols accessed via module aliases may not resolve

---

## 9. No Incremental Re-indexing

**Limitation:** When a file changes, the entire file is re-indexed. There is no incremental update based on what changed.

**Impact:**
- Large files may take longer to re-index
- More work done than strictly necessary

**Why:** Tree-sitter supports incremental parsing, but the symbol extraction would need significant changes to take advantage of it. For typical file sizes, full re-extraction is fast enough.

---

## Summary

fsharp-tools is designed as a **lightweight, fast, predictable** tool for F# navigation—not a replacement for the full F# language server. It excels at:

- Fast codebase-wide symbol search
- Go-to-definition for user-defined symbols
- Bounded, predictable memory usage
- AI agent integration via CLI

For full type-aware analysis, refactoring, and external dependency navigation, use fsautocomplete or another full-featured F# language server.