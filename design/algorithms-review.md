# Algorithmic and Architectural Review of `fsharp-tools`

## Overview

This document reviews the algorithms and data structures used in `fsharp-tools`, and compares them to those used by other language servers and code indexers. The goal is to evaluate whether the project is using appropriate approaches for its stated goals, and to identify future improvements if we ever want to expand the feature set.

**Scope:**

- Focus on architecture and algorithms, not specific implementation bugs.
- Compare to common patterns in:
  - Language servers (fsautocomplete, rust-analyzer, tsserver, clangd/ccls).
  - Large-scale code indexers (Sourcegraph, Kythe, etc.).
- Assume the current goals from `README.md`:
  - Minimal, lightweight F# language server.
  - Bounded memory (<50MB on large codebases).
  - Fast indexing and navigation.
  - No .NET runtime, no type-aware analysis.
  - Optimized for AI-assisted workflows.

---

## 1. High-Level Architecture

### Current structure

From the repo layout and crate APIs:

- `crates/fsharp-index`  
  Core library for symbol extraction, indexing, name resolution, and dependency graph traversal.

- `crates/fsharp-cli`  
  CLI front-end over `fsharp-index`:
  - Build/rebuild index.
  - Update index.
  - Go to definition.
  - Find references.
  - Spider/dependency graph traversal.
  - Symbol search.

- `crates/fsharp-lsp`  
  Language Server Protocol implementation over `fsharp-index`.

- `crates/fsharp-index::watch`  
  Filesystem watcher and helpers for incremental indexing.

### Comparison with other systems

Broadly similar conceptual layering:

- `rust-analyzer`  
  Core incremental database (HIR + salsa) + LSP front-end.

- TypeScript language server (`tsserver`)  
  Project graph + incremental program + language service + LSP/VSCode integration.

- `clangd` / `ccls`  
  Clang-based parsing + persistent symbol index + background indexer + LSP.

- Indexers like Sourcegraph, Kythe  
  Language-specific indexers that emit a persistent graph, with separate serving/search components.

**Key difference:** `fsharp-tools` explicitly trades away type-system integration and heavy semantics in favor of:

- Bounded, predictable memory.
- Simple data structures (hash maps, vectors).
- A purely syntactic + contextual analysis stack built on tree-sitter.

**Conclusion:**  
Given the stated goals, the high-level architecture is well-aligned with how modern tooling is structured, just with a deliberately thinner semantic layer.

---

## 2. Parsing and Symbol Extraction (`parse.rs` and `lib.rs`)

### Current approach

`fsharp-index` defines:

- `extract_symbols(file, source) -> ExtractionResult` (re-exported from `parse`):

  ```text
  ExtractionResult {
      symbols: Vec<Symbol>,
      references: Vec<Reference>,
      opens: Vec<String>,
      module: Option<String>,
  }
  ```

- `Symbol` includes:
  - `name` (short name).
  - `qualified` (e.g., `MyApp.Services.PaymentService.processPayment`).
  - `kind` (`SymbolKind` enum: Module, Function, Value, Type, Record, Union, Interface, Class, Member).
  - `location` (`Location` with file, line/column start and end).
  - `visibility` (`Visibility` enum: Public/Internal/Private).

- Parsing/extraction is built on **tree-sitter for F#** and uses a recursive traversal:
  - `extract_recursive`, `extract_recursive_with_depth`
  - `extract_type_defn`, `extract_members`
  - `handle_function_or_value_defn`
  - `qualified_name`, `extract_visibility`, `is_reference_context`
  - `node_to_location`, `find_child_by_kind`

These functions walk the tree-sitter CST to extract:

- Declarations (symbols).
- References (identifier uses) in syntactic reference contexts.
- `open` statements and module/namespace paths.

### Comparison to other language servers

Other servers / tools commonly:

- Use incremental parsers (including tree-sitter) to:
  - Maintain a persistent parse tree per file.
  - Apply incremental edits for local changes.
- Build an intermediate representation on top of the parse:
  - `rust-analyzer`: HIR (High-Level IR) + name resolution + type inference.
  - TypeScript: AST + `Symbol` graph + type checker + project graph.
  - Clang-based tools: Clang AST + symbol index, with type and template information.

`fsharp-tools`:

- Uses tree-sitter, but (as currently wired through the CLI) typically reparses the entire file when indexing.
- Works directly with the CST for extraction instead of building a separate high-level IR.

### Assessment

**Strengths:**

- Using tree-sitter is a very good modern choice:
  - Editor-friendly, incremental-capable parser.
  - Avoids F# compiler services and .NET runtime.
- Hand-written extraction logic is simple and predictable:
  - Straightforward to debug and extend for more syntactic constructs.
  - No heavy framework or complex IR layer.

**Tradeoffs:**

- Today, the indexer appears to perform full-file re-parses during `build`/`update`, instead of incremental edits:
  - Acceptable for small-to-medium projects and CLI usage.
  - For near-real-time response in large projects, incremental parsing per open file would be a useful improvement.
- Reference detection is syntactic:
  - `is_reference_context` decides which identifier nodes count as references.
  - No type-driven disambiguation or binding-level information.

**Conclusion:**  
For a minimal server that aims for fast symbol navigation, tree-sitter-based syntactic extraction is a solid, modern choice. The next algorithmic step would be incremental per-file parsing (especially for open documents), not a new parser technology.

---

## 3. Index Structure and Core Algorithms (`index.rs`)

### Data structures

From the outline and APIs, `CodeIndex` contains:

- `workspace_root`
- `definitions`
- `file_symbols`
- `file_references`
- `module_files`
- `file_opens`

And exposes methods:

- Construction & configuration:
  - `new`, `with_root`, `set_workspace_root`, `workspace_root`
  - `make_location_absolute`, and internal `to_relative`/`to_absolute`.
- Mutation:
  - `add_symbol(Symbol)`
  - `add_reference(PathBuf, Reference)`
  - `add_open(PathBuf, String)`
  - `clear_file(PathBuf)` (remove all data about a file).
- Queries:
  - `get(name)`
  - `get_all(name)` (likely all overloads/variants)
  - `get_absolute(name)` (returns locations with absolute paths)
  - `symbols_in_file(file)`
  - `references_in_file(file)`
  - `opens_for_file(file)`
  - `find_references(symbol_qualified_name)`
  - `search(pattern)`
  - `symbol_count`, `file_count`, `files`, `contains_file`

Tests cover:

- Adding and retrieving symbols.
- Per-file symbol queries.
- Search behavior.
- `find_references` and reference lookups.
- Overloading and shadowing behavior across files.

From this, the natural internal layout is some combination of:

- `definitions: HashMap<String, Vec<Symbol>>` keyed by qualified or short names.
- `file_symbols: HashMap<PathBuf, Vec<Symbol>>`.
- `file_references: HashMap<PathBuf, Vec<Reference>>`.
- `file_opens: HashMap<PathBuf, Vec<String>>`.
- `module_files: HashMap<String, Vec<PathBuf>>`.

### Comparison to other systems

More “industrial” indexers tend to use:

- Symbol IDs (USRs, GUIDs, or interned IDs) instead of names as primary keys:
  - Clang/clangd/ccls: USRs (Universal Symbol Resolution).
  - Rust: stable IDs for HIR items.
- Disk-backed / sharded indices for very large codebases:
  - Inverted indices for fast search, compressed on-disk formats.
- Fine-grained incremental invalidation:
  - Rebuilding only affected parts of the index when a file changes, based on dependency graphs.

`fsharp-tools` takes a simpler, in-memory + JSON persistence approach:

- Hash-map based indices.
- Symbol identity is textual (qualified name) plus location and kind.
- Entire index is serialized to a single JSON file.

### Complexity and performance

Assuming straightforward `HashMap` and `Vec` based design:

- Lookup by qualified name: **O(1)** average (`HashMap`).
- Search by pattern: **O(N)** in number of symbols.
- Per-file symbol/references queries: **O(k)** where k is symbols/refs in that file.
- Clear & re-add for a file:
  - **O(k)** for that file’s data plus some overhead to update global maps.

This is more than adequate for:

- Single-project use.
- Typical F# repo sizes (tens of thousands of symbols).
- Occasional `build`/`update` operations.

For monorepos or millions of symbols, improvements might be warranted:

- More compact symbol IDs.
- Inverted indices for substring/wildcard search.
- Better on-disk representation than pretty-printed JSON.

### Conclusion

The current indexing algorithms and data structures are:

- **Appropriate and efficient** for the project’s current scale and goals.
- Very simple to reason about and maintain.
- A good foundation for incremental improvements (per-file rebuilds, partial invalidation) without needing large architectural changes.

The “optimal” techniques used by huge systems (Kythe, Sourcegraph, clangd’s global index) are not necessary at this scale and would add substantial complexity for little immediate benefit.

---

## 4. Name Resolution (`resolve.rs`)

### Current algorithms

`CodeIndex` implements F#-style name resolution with methods:

- `resolve(name: &str, from_file: &Path) -> Option<ResolveResult>`
- `resolve_dotted(name: &str, from_file: &Path) -> Option<ResolveResult>`

`ResolveResult` includes:

- `symbol: &Symbol`
- `resolution_path: ResolutionPath` (Qualified, ViaOpen(module), SameModule, ParentModule(module)).

The algorithm for `resolve`:

1. **Exact qualified name match**  
   - `get(name)` as a fully-qualified symbol name.

2. **Same-file resolution**  
   - Look at `symbols_in_file(from_file)`:
     - Match on `symbol.name == name`, or
     - `symbol.qualified.ends_with("." + name)`.

3. **Via open statements**  
   - For each `open_module` in `opens_for_file(from_file)`:
     - Try `open_module + "." + name`.
     - For dotted `name` like `List.map`, also try `open_module + "." + name` (i.e., `open_module.List.map`).

4. **Parent modules**  
   - From file’s symbols, infer the module path from their qualified names.
   - For each module path, progressively walk up (`A.B.C` → `A.B` → `A`) and try `module + "." + name`.

`resolve_dotted`:

1. Try `resolve(name)` directly.
2. If `name` contains `.`, split into first component and rest:
   - Let `module_name = parts[0]`, `member_name = parts[1]`.
   - For each open module:
     - If `open_module` ends with `module_name`, try `open_module + "." + member_name`.
     - Also try `open_module + "." + module_name + "." + member_name`.

This matches many typical F# patterns:

- Resolving `helper` in a file that `open MyApp.Utils`.
- Resolving `List.map` via `open FSharp.Collections`.
- Finding symbols in the same file, then via opens, then via parent modules.

### Comparison to other language servers

- `fsautocomplete`:
  - Delegates to F# compiler services → full type-checker-based name resolution.
  - Handles:
    - Types, generics, overload resolution.
    - Active patterns, computation expressions, etc.

- `rust-analyzer`:
  - Rich name resolution:
    - Lexical scopes, module system, prelude, imports, macros.
  - Integrated with HIR and type inference.

- `tsserver`:
  - Symbol graph-based resolution with object types, generics, module systems, etc.

`fsharp-tools` does **purely syntactic & context-based** resolution:

- No type-level information at all.
- No overload resolution by signature or type.
- No inference for implicit imports beyond what is expressible with modules/opens.

### Assessment

**What works well:**

- The algorithm is structured similarly to “real” language servers:
  - Try qualified first, then local file, then opens, then parent modules.
- It handles common F# name resolution cases using only:
  - Module/namespace paths,
  - `open` declarations,
  - Per-file symbols.

**What’s missing (by design):**

- Type-based disambiguation when multiple symbols share the same name:
  - E.g., overloads, methods in different types, extension members.
- Understanding of:
  - Generic type parameters, constraints.
  - Active patterns, computation expressions.
  - Symbols imported indirectly via other modules or type providers.

Given `fsharp-tools` explicitly does not aim for type-aware completions/hover, the choice of a “best-effort” syntactic resolver is consistent and reasonable.

### Conclusion

Algorithmically, `resolve` follows the right conceptual steps for F#-like name resolution with the information available (symbols, modules, opens), but intentionally stops short of full semantic analysis.

For its goals—fast goto-def and navigation for AI tooling without a compiler dependency—this is a **good tradeoff**. A “best possible” resolver would require integrating F# compiler services or implementing a substantial subset of the F# type system, which is explicitly out of scope.

---

## 5. Dependency Graph Traversal / Spider (`spider.rs`)

### Current algorithms

`spider(index: &CodeIndex, entry_point: &str, max_depth: usize) -> SpiderResult`:

- Data structures:
  - `SpiderNode { symbol: Symbol, depth: usize }`.
  - `SpiderResult { nodes: Vec<SpiderNode>, unresolved: Vec<String> }`.
  - `visited: HashSet<String>` for qualified symbol names.
  - BFS queue: `VecDeque<(String, usize)>`.

- Algorithm (BFS):

  1. Seed queue with `(entry_point, 0)`.
  2. While queue non-empty:
     - Pop `(qualified_name, depth)`.
     - Skip if `qualified_name` already in `visited`.
     - Lookup symbol via `index.get(&qualified_name)`.
       - If found:
         - Append `SpiderNode { symbol.clone(), depth }` to result.
         - If `depth >= max_depth`, continue to next.
         - Gather:
           - `references_in_file(symbol.location.file)`.
           - `opens_for_file(symbol.location.file)`.
         - For each reference `r`:
           - Try to resolve via `try_resolve_reference(index, &r.name, opens)`.
             - If resolved to `resolved_name` and not visited:
               - Push `(resolved_name, depth + 1)` onto the queue.
             - If unresolved:
               - Track in `result.unresolved` (dedup).
       - If not found and `depth == 0`, record the entry point as unresolved.

`try_resolve_reference`:

1. Direct match: `index.get(name)`.
2. Via opens: for each `open`, try `open + "." + name`.
3. Partial match fallback:
   - `index.search(name)`.
   - Use first search match’s qualified name.

`spider_from_file(index, file, max_depth)`:

- Enumerate `symbols_in_file(file)` as entry points.
- Call `spider` for each and merge results (deduping visited symbols and unresolved names).

### Comparison to other tools

The spider is essentially a “lightweight call/dependency graph”:

- Very similar in spirit to:
  - Call hierarchy in language servers.
  - Reachability analysis in static analysis tools.
- More advanced tools use:
  - Type-aware resolution for calls.
  - Distinction between different kinds of references (call, type use, field access, etc.).
  - Potentially call graph + control-flow analysis.

`fsharp-tools`:

- Uses syntactic references from `parse.rs` and the name resolver / search to approximate dependency edges.
- Uses BFS with depth limiting and a visited-set, which is exactly the right basic algorithm for reachability graphs.

### Assessment

**Algorithmically sound parts:**

- BFS with a visited-set is the correct traversal method for a dependency/call graph:
  - Avoids cycles.
  - Depth limit is easy to enforce.
- Splits results into:
  - `nodes` in BFS order with depths.
  - `unresolved` references that could not be mapped to any symbol.

**Heuristic/weak spots:**

- `try_resolve_reference` falls back to `index.search(name)` and picks the first result:
  - This is purely heuristic.
  - In large codebases with many identically-named symbols, the spider may follow the wrong path.
  - For exploratory analysis and AI-driven workflows, this can still be quite useful, but it is not a precise call graph.

**Conclusion**

Given the constraints (no type system, syntactic references only), the spider’s algorithm is about as good as it can reasonably be without adding a lot more semantic machinery.

The traversal algorithm itself (BFS, visited-set, `max_depth`) is textbook, and the main improvement area is the accuracy of reference resolution, not the traversal.

---

## 6. File Watching and Incremental Updates (`watch.rs` and CLI `cmd_update`)

### Current algorithms

`watch.rs` provides:

- `FileWatcher`:
  - Uses `notify` crate with a configurable poll interval.
  - Watches a root directory recursively.
  - Filters events down to F# source files via `is_fsharp_file(path)`:
    - `.fs`, `.fsi`, `.fsx`.
  - Emits `WatchEvent`:
    - `Created(PathBuf)`, `Modified(PathBuf)`, `Deleted(PathBuf)`, `Renamed(PathBuf, PathBuf)`.
  - Offers:
    - `poll()` (non-blocking).
    - `wait()` (blocking).
    - `wait_timeout(Duration)`.

- `find_fsharp_files(root)`:
  - Recursive filesystem walk.
  - Skips typical non-source directories:
    - `.`-prefixed, `node_modules`, `bin`, `obj`, `packages`.

`fsharp-cli`’s `cmd_update` currently does:

- Load existing index from `.fsharp-index/index.json`.
- Set `workspace_root`.
- **Re-scan all F# files** via `find_fsharp_files(root)`.
- For each file:
  - Re-read source.
  - Call `extract_symbols`.
  - `clear_file(file)` then `add_symbol` / `add_reference` / `add_open`.

The code comments explicitly note that this is a simplification:

> // TODO: Use file modification times or a proper incremental strategy

### Comparison to other language servers

Other language servers typically:

- Maintain per-file analysis state and parse trees.
- Recompute only for changed files:
  - For a changed file:
    - Reparse (often incrementally).
    - Recompute symbols and references for that file.
    - Update global indices.
- Use dependency graphs to propagate invalidation selectively.

`fsharp-tools`:

- Has the building blocks:
  - `clear_file(file)` for removal.
  - `add_*` for incremental rebuild per file.
  - A file watcher to detect changes.
- But the `cmd_update` CLI currently opts for a full re-scan and re-index.

### Assessment

**Current behavior:**

- Algorithmically simple and robust:
  - No subtle incremental edge cases.
  - Suitable if `update` is not called on every single keystroke, but on demand or via coarser-grained triggers.
- Cost is O(#files) per `update` call, with each file re-parsed and re-extracted.

**Potential evolution:**

- Use the `FileWatcher` and/or file modification timestamps to:
  - Build a list of changed/created/deleted files.
  - Run `clear_file` and re-extract only for those.
- Optionally integrate with tree-sitter’s incremental parsing:
  - For open files, maintain a parse tree and apply edits, then re-run a local extraction traversal instead of full parse.

**Conclusion**

The **algorithms for incremental updates exist** (per-file clear and re-add, plus file watcher), but the CLI currently uses the simplest possible implementation—full rebuild on `update`. This is not an algorithmic dead-end; it’s an intentionally conservative initial strategy with a clear path to incrementalization.

---

## 7. LSP Layer (`fsharp-lsp`)

Even without diving into implementation details, the expected flow is:

- On LSP requests like:
  - `textDocument/definition`
  - `textDocument/references`
- The server would:
  - Map LSP position → file → nearest symbol/reference (using `CodeIndex` and/or a per-document map).
  - Use:
    - `CodeIndex::resolve` / `resolve_dotted` for definitions.
    - `CodeIndex::find_references` for references.
    - Possibly `spider` for custom graph-like features.

Compared to richer language servers:

- `fsharp-tools` intentionally does not:
  - Provide type-aware completions.
  - Offer rich hover type info.
  - Run full compiler passes in the background.

This is by design and helps stay within the bounded memory and complexity budget.

---

## 8. Overall Evaluation: Are We Using “The Best” Algorithms?

### Against our own goals

Given the project’s stated goals:

- Minimal, bounded-memory F# language server.
- Syntactic, not type-based, semantics.
- Optimized for indexing/navigation + AI tooling.
- No .NET runtime dependency.

The current algorithmic and architectural choices are **well chosen and coherent**:

- **Parsing & extraction:**  
  - tree-sitter-based CST traversal with custom extraction logic is modern and appropriate.
- **Index:**  
  - Hash-map based, in-memory index with simple JSON persistence is efficient and maintainable.
- **Name resolution:**  
  - Approximate but F#-aware resolution using symbols, modules, and opens, mirroring the broad structure of full resolvers without their complexity.
- **Spider:**  
  - Breadth-first traversal over a syntactic reference graph is the right high-level algorithm; the heuristic fallback for ambiguous references is a conscious tradeoff.
- **Incremental updates:**  
  - Foundational pieces are present; current CLI favors simplicity over optimal incremental performance.

### Where “the best possible” (theoretically) diverges

If we define “best” purely in terms of precision and capability (not within current constraints), we would be looking at:

1. **Type-aware resolution and indexing**
   - Integration with F# compiler services or a self-hosted type system.
   - Precise, type-directed name resolution, overload selection, and reference classification.

2. **Richer intermediate representation**
   - An IR/HIR layer above the CST to model:
     - Modules, namespaces, types, and members consistently.
     - Scopes and symbol tables.
     - Possibly data/control flow.

3. **Sophisticated indexing structures**
   - Symbol IDs (USRs) rather than textual names.
   - On-disk or distributed indices.
   - Inverted indices for fast substring/glob search.

4. **Fully incremental analysis**
   - Per-file incremental parsing and re-analysis.
   - Dependency-aware invalidation of derived data.

All of those are **deliberately outside** the current project scope and would pull us towards a fsautocomplete-class tool, which defeats the minimalism and bounded-memory goals.

### Summary

Within the project’s intentionally constrained design space:

- The core algorithms are **strong and modern** (tree-sitter, BFS, hash maps).
- The resolution and spider logic follow the same *shapes* as richer tools, just operating on a syntactic and contextual basis.
- The main opportunities for improvement are incrementalization and scaling behaviors, not fundamental algorithmic redesign.

If we decide to expand the feature set or target very large monorepos in the future, the natural next algorithmic steps would be:

1. **Incremental per-file indexing** driven by `FileWatcher` and modified file lists.
2. **Incremental parsing** for open documents using tree-sitter edits.
3. **More precise reference resolution** (e.g., store resolved symbol IDs per reference when possible).
4. **Better search index** for symbol name search (suffix arrays, n-gram indexes, etc.).
5. Optional **type-system integration** as a separate “full-fidelity” mode distinct from the current lightweight mode.

For the current goals, though, the system is using **appropriate and effective algorithms**, and there are no glaring algorithmic missteps that prevent it from being a high-quality minimal F# indexer and language server.