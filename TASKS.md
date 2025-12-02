# fsharp-tools: Daily Driver Task List

**Goal:** Transform this proof-of-concept into a reliable daily driver for F# development in Zed.

**Status:** ✅ Core features complete! Ready for daily use.

---

## Priority 1: Critical UX Fixes

### 1.1 ✅ Fix Location to Include End Positions
**Status:** Complete  
**Effort:** Low (1-2 hours)  
**Impact:** High

The `Location` struct only stores start position, causing the LSP to hack in a `+10` character offset for the end position. This results in broken highlighting in the editor.

**Tasks:**
- [x] Add `end_line: u32` and `end_column: u32` to `Location` struct in `lib.rs`
- [x] Update `node_to_location()` in `parse.rs` to use `node.end_position()`
- [x] Update `to_lsp_location()` in `fsharp-lsp/src/main.rs` to use actual end positions
- [x] Update all tests that construct `Location` manually
- [x] Update serialization (index format version bump if needed)

**Files:**
- `crates/fsharp-index/src/lib.rs`
- `crates/fsharp-index/src/parse.rs`
- `crates/fsharp-lsp/src/main.rs`

---

### 1.2 ✅ Implement In-Memory Document Store
**Status:** Complete  
**Effort:** Medium (2-4 hours)  
**Impact:** High

The server ignores `didChange` events and only works with saved files. This means:
- New symbols aren't available until save
- Line numbers may be stale, causing jumps to wrong locations

**Tasks:**
- [x] Create `DocumentStore` struct to hold in-memory file contents
- [x] Update `did_open` to store document content
- [x] Update `did_change` to apply incremental changes
- [x] Update `did_close` to remove document from store
- [x] Update `get_symbol_at_position` to read from store instead of disk
- [x] Consider: reindex on change (debounced) vs only on save

**Files:**
- `crates/fsharp-lsp/src/main.rs` (or new `document_store.rs`)

---

## Priority 2: Feature Completeness

### 2.1 Add Hover Support
**Status:** Complete  
**Effort:** Low (1-2 hours)  
**Impact:** Medium

We already have resolution logic. Showing symbol info on hover is low-hanging fruit.

**Tasks:**
- [x] Implement `textDocument/hover` handler
- [x] Return symbol qualified name, kind, and file location
- [x] Format as Markdown for nice display

**Files:**
- `crates/fsharp-lsp/src/main.rs`

---

### 2.2 Add Find References
**Status:** Complete  
**Effort:** Medium (2-3 hours)  
**Impact:** Medium

We already index references. Expose them via LSP.

**Tasks:**
- [x] Implement `textDocument/references` handler
- [x] Query index for all references to the symbol at cursor
- [x] Return list of locations
- [x] Add `find_references` method to `CodeIndex`

**Files:**
- `crates/fsharp-lsp/src/main.rs`
- `crates/fsharp-index/src/index.rs` (may need helper method)

---

## Priority 3: Robustness

### 3.1 Handle Symbol Overloading/Shadowing
**Status:** Complete  
**Effort:** Medium (2-3 hours)  
**Impact:** Medium

The `definitions` HashMap uses qualified name as key, so overloaded methods clobber each other.

**Tasks:**
- [x] Change `definitions: HashMap<String, Symbol>` to `HashMap<String, Vec<Symbol>>`
- [x] Update `add_symbol`, `get`, `resolve` methods
- [x] For resolution, prefer closest/most-recent definition
- [x] Update serialization
- [x] Add `get_all()` method for accessing all overloads

**Files:**
- `crates/fsharp-index/src/index.rs`
- `crates/fsharp-index/src/resolve.rs`

---

### 3.2 Improve Error Recovery
**Status:** Complete  
**Effort:** Low (1 hour)  
**Impact:** Low

Handle Tree-sitter ERROR nodes gracefully to avoid garbage symbols.

**Tasks:**
- [x] Check for `node.is_error()` in `extract_recursive`
- [x] Skip error nodes or log warning
- [x] Also skip `is_missing()` nodes (parser recovery artifacts)

**Files:**
- `crates/fsharp-index/src/parse.rs`

---

## Priority 4: Performance (Future Optimization)

### 4.1 Optimize Workspace Symbol Search
**Status:** Deferred  
**Effort:** Medium (3-4 hours)  
**Impact:** Medium (for large codebases)

Linear scan over all definitions will be slow for 10k+ symbols.

**Tasks:**
- [ ] Add a secondary index: prefix trie or ngram index
- [ ] Benchmark current performance on large codebase
- [ ] Implement and compare

**Files:**
- `crates/fsharp-index/src/index.rs`

---

### 4.2 Cache Parse Trees
**Status:** Deferred  
**Effort:** Medium (2-3 hours)  
**Impact:** Medium

Currently re-parsing file on every `goto_definition` call.

**Tasks:**
- [ ] Store parsed tree alongside document content
- [ ] Invalidate on document change
- [ ] Use cached tree for symbol lookup

**Files:**
- `crates/fsharp-lsp/src/main.rs`

---

## Completed

### 3.2 ✅ Improve Error Recovery (2024-01-XX)
- Added `node.is_error()` check in `extract_recursive_with_depth`
- Skip ERROR nodes to avoid garbage symbols from malformed code
- Also skip `is_missing()` nodes (parser recovery artifacts)
- Added debug logging for skipped error nodes

### 3.1 ✅ Handle Symbol Overloading/Shadowing (2024-01-XX)
- Changed `definitions` from `HashMap<String, Symbol>` to `HashMap<String, Vec<Symbol>>`
- `get()` returns the most recently added symbol (for shadowing semantics)
- Added `get_all()` to retrieve all overloads
- Updated `clear_file()` to properly handle multi-symbol entries
- Updated `symbol_count()`, `search()`, `symbols_in_file()`, `find_references()`
- Added tests for overloading and cross-file shadowing

### 2.2 ✅ Add Find References (2024-01-XX)
- Added `find_references` method to `CodeIndex` that searches across all files
- Implemented `textDocument/references` LSP handler
- Matches short names, qualified names, and partial qualified names

### 2.1 ✅ Add Hover Support (2024-01-XX)
- Implemented `textDocument/hover` handler
- Returns formatted Markdown with symbol kind, name, location, and qualified name

### 1.2 ✅ Implement In-Memory Document Store (2024-01-XX)
- Created `DocumentStore` module with `Document` struct for tracking open files
- Implemented incremental text change application (LSP `didChange`)
- Updated `did_open`, `did_change`, `did_close` handlers
- `get_symbol_at_position` now reads from in-memory store first, falls back to disk
- Full test coverage for position-to-offset conversion and change application

### 1.1 ✅ Fix Location to Include End Positions (2024-01-XX)
- Added `end_line` and `end_column` fields to `Location` struct
- Updated `node_to_location()` to capture tree-sitter end positions
- Removed the `+10` hack from `to_lsp_location()` - now uses actual end positions
- Fixed all test helpers to use `Location::new()` constructor

---

## Summary

### Features Implemented
- ✅ **Go-to-definition** with accurate symbol highlighting (end positions)
- ✅ **Find references** across workspace
- ✅ **Hover** with symbol info (kind, qualified name, location)
- ✅ **Workspace symbol search** (Cmd+T)
- ✅ **In-memory document tracking** for unsaved files
- ✅ **Incremental text sync** (LSP didChange)
- ✅ **Symbol overloading/shadowing** support
- ✅ **Error recovery** from malformed F# code

### Notes

- All Priority 1-3 tasks complete
- Priority 4 (performance) deferred until profiling shows need
- Run `cargo test --all` after any changes
- Test manually in Zed with real F# projects