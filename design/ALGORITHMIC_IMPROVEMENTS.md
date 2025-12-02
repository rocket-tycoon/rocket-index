# Algorithmic Improvements Task List

Based on the algorithmic review of fsharp-tools, this document tracks improvements that would enhance correctness and performance.

---

## Priority 1: Correctness Improvements

### 1.1 F# Compilation Order Awareness from `.fsproj`

**Status:** ✅ Complete  
**Effort:** Medium (4-6 hours)  
**Impact:** High (correctness)

F# files have a defined compilation order in `.fsproj` files. Files earlier in the order cannot reference symbols from files later in the order. The current implementation ignores this entirely, which can lead to:
- Incorrect symbol resolution (resolving to a symbol that wouldn't be visible at compile time)
- Inaccurate spider dependency graphs
- False positives in "find references"

**Tasks:**
- [x] Add `.fsproj` XML parsing (use `quick-xml` or `roxmltree` crate)
- [x] Extract `<Compile Include="..."/>` elements in order
- [x] Store file compilation order in `CodeIndex`
- [x] Use compilation order in resolution: earlier files take precedence
- [x] Update spider to respect compilation order (can't depend on later files)
- [x] Add fallback behavior when no `.fsproj` is found (current behavior)

**Files to modify:**
- `crates/fsharp-index/Cargo.toml` - add XML parsing dependency
- `crates/fsharp-index/src/lib.rs` - add `fsproj` module
- `crates/fsharp-index/src/fsproj.rs` - new file for .fsproj parsing
- `crates/fsharp-index/src/index.rs` - add file order tracking
- `crates/fsharp-index/src/resolve.rs` - use file order in resolution
- `crates/fsharp-cli/src/main.rs` - parse .fsproj during build

**Example .fsproj structure:**
```xml
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <Compile Include="Types.fs" />
    <Compile Include="Utils.fs" />
    <Compile Include="Services.fs" />
    <Compile Include="Program.fs" />
  </ItemGroup>
</Project>
```

---

### 1.2 Document Known Limitations

**Status:** ✅ Complete  
**Effort:** Low (1 hour)  
**Impact:** Medium (user expectations)

Users should understand what this tool does and doesn't do.

**Tasks:**
- [x] Add "Known Limitations" section to README.md
- [x] Create `design/KNOWN_LIMITATIONS.md` with detailed explanation

**Limitations to document:**
- No type-aware analysis (tree-sitter only, no .NET runtime)
- Compilation order not currently respected (until 1.1 is done)
- O(N) workspace symbol search (fine for typical project sizes)
- Spider dependency graph is approximate (best-effort resolution)
- No understanding of F# type inference or overload resolution

---

## Priority 2: Performance Improvements

### 2.1 Parallel File Parsing with Rayon

**Status:** ✅ Complete  
**Effort:** Low (2-3 hours)  
**Impact:** Medium (indexing speed)

Initial indexing parses files sequentially. This is easy to parallelize.

**Tasks:**
- [x] Add `rayon` dependency to `fsharp-index` and `fsharp-cli`
- [x] Use `par_iter()` for file parsing during index build
- [x] Collect results and merge into single `CodeIndex`
- [ ] Benchmark before/after on a real codebase

**Files to modify:**
- `crates/fsharp-index/Cargo.toml` - add rayon
- `crates/fsharp-cli/Cargo.toml` - add rayon
- `crates/fsharp-cli/src/main.rs` - parallelize `cmd_build`

**Expected code pattern:**
```rust
use rayon::prelude::*;

let results: Vec<_> = files
    .par_iter()
    .filter_map(|file| {
        let source = std::fs::read_to_string(file).ok()?;
        Some((file.clone(), extract_symbols(&source, file)))
    })
    .collect();

for (file, parse_result) in results {
    // merge into index (sequential, index is not thread-safe)
}
```

---

## Priority 3: Future Optimizations (Deferred)

These are documented for future consideration but not prioritized for implementation.

### 3.1 Optimize Workspace Symbol Search

**Current:** O(N) linear scan with string allocations on every query  
**Alternative:** Prefix trie, ngram index, or `fst` crate  
**Why deferred:** Performance is acceptable for typical F# project sizes (<10k symbols)

### 3.2 String Interning

**Current:** Each symbol stores full String copies  
**Alternative:** Use `string_interner` crate or custom intern table  
**Why deferred:** Memory usage is within goals (<50MB), adds complexity

### 3.3 Binary Serialization

**Current:** JSON serialization with `serde_json`  
**Alternative:** `bincode` for faster, more compact serialization  
**Why deferred:** JSON is debuggable and human-readable, which is valuable during development

### 3.4 Spider Resolution Improvement

**Current:** Falls back to first search result when resolution fails  
**Alternative:** Return ambiguous/unresolved instead of guessing  
**Why deferred:** Spider is a secondary feature, current behavior is "good enough"

---

## Implementation Order

1. ~~**1.2 Document Known Limitations** - Quick win, sets expectations~~ ✅
2. ~~**2.1 Parallel File Parsing** - Easy performance improvement~~ ✅
3. ~~**1.1 F# Compilation Order** - Biggest correctness improvement~~ ✅

---

## Notes

- All changes should maintain the project's goals: minimal, bounded-memory, no .NET runtime
- Run `cargo test --all` after any changes
- Benchmark on real F# projects when making performance claims