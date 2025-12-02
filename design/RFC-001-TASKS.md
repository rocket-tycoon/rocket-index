# RFC-001 Implementation Task List: Hybrid Type Architecture

**RFC:** [RFC-001-hybrid-type-architecture.md](./RFC-001-hybrid-type-architecture.md)  
**Status:** In Progress  
**Created:** 2024-12-02  
**Updated:** 2024-12-03 (CLI migrated to SQLite)
**Target:** Phase 1 completion in ~3-4 weeks

---

## Overview

This task list breaks down the Hybrid Type Architecture RFC into implementable work items. 

**Major Change:** The RFC has been updated to use **SQLite** instead of JSON for index storage. This provides indexed O(log n) lookups, low memory usage (query on-demand), incremental updates, and rich query capabilities.

**Legend:**
- 🔴 Blocked
- 🟡 In Progress  
- 🟢 Complete
- ⬜ Not Started

---

## Completed Work (Pre-SQLite)

The following work was completed before the SQLite decision and remains useful:

### ✅ Task 1.1: Type Domain Models
- Created `type_cache.rs` with domain types
- `TypedSymbol`, `TypeMember`, `MemberKind`, `ParameterInfo` structs
- Serde serialization (still useful for JSON export/import)
- 22 unit tests passing

### ✅ Task 1.2: TypeCache Runtime Structure  
- `TypeCache` struct with HashMap-based lookups
- `get_type()`, `get_members()`, `get_member()` methods
- JSON loading and parsing

### ✅ Task 1.3: CodeIndex Integration
- Added `type_cache` field to `CodeIndex`
- `load_type_cache()`, `set_type_cache()`, `has_type_cache()` methods
- `get_symbol_type()`, `get_type_members()`, `get_type_member()` delegating methods

### ✅ Task 2.1: Type-Aware Resolution
- Added `ResolutionPath::ViaMemberAccess` variant
- `MemberResolveResult` struct
- `infer_expression_type()`, `resolve_member_access()`, `resolve_with_type_info()` methods
- `extract_simple_type()` helper for parsing type signatures
- 21 unit tests for resolution

### ✅ Task 3.1: F# Extraction Script (Partial)
- Created `scripts/extract-types.fsx` 
- Uses FSharp.Compiler.Service
- Extracts symbols and type members
- **Needs update:** Currently writes JSON, should write to SQLite

---

## Phase 1: SQLite Migration & Type Extraction

### Milestone 1: SQLite Foundation

#### 1.1 🟢 Add rusqlite Dependency
**Effort:** 30 minutes  
**Dependencies:** None  
**Files:** `Cargo.toml`, `crates/fsharp-index/Cargo.toml`

**Completed:**
- [x] Add `rusqlite = { version = "0.31", features = ["bundled"] }` to workspace dependencies
- [x] Add rusqlite to fsharp-index dependencies
- [x] Verify builds successfully

---

#### 1.2 🟢 Create SQLite Schema Module
**Effort:** 2-3 hours  
**Dependencies:** 1.1  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Create `db.rs` module
- [x] Define schema constants matching RFC (symbols, members, refs, opens, metadata)
- [x] Implement `init_schema()`
- [x] Implement `get_schema_version()`
- [x] Implement `set_metadata()` / `get_metadata()`

---

#### 1.3 🟢 Implement SqliteIndex Struct
**Effort:** 4-5 hours  
**Dependencies:** 1.2  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Create `SqliteIndex` struct wrapping `Connection`
- [x] Implement `SqliteIndex::create(path: &Path) -> Result<Self>`
- [x] Implement `SqliteIndex::open(path: &Path) -> Result<Self>`
- [x] Implement `SqliteIndex::open_or_create(path: &Path) -> Result<Self>`
- [x] Implement `SqliteIndex::in_memory()` for testing
- [x] Schema version validation on open

---

### Milestone 2: Symbol Storage (SQLite)

#### 2.1 🟢 Symbol Write Operations
**Effort:** 3-4 hours  
**Dependencies:** 1.3  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Implement `insert_symbol(&self, symbol: &Symbol) -> Result<i64>`
- [x] Implement `insert_symbol_with_type()` for type signatures
- [x] Implement `insert_symbols(&self, symbols: &[Symbol]) -> Result<()>` batch insert
- [x] Implement `delete_symbols_in_file(&self, file: &Path) -> Result<usize>`
- [x] Transaction support via `begin_transaction()`

---

#### 2.2 🟢 Symbol Read Operations  
**Effort:** 3-4 hours  
**Dependencies:** 2.1  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Implement `find_by_qualified(&self, qualified: &str) -> Result<Option<Symbol>>`
- [x] Implement `find_all_by_qualified()` for overloads
- [x] Implement `search(&self, pattern: &str, limit: usize) -> Result<Vec<Symbol>>`
- [x] Implement `symbols_in_file(&self, file: &Path) -> Result<Vec<Symbol>>`
- [x] Implement `count_symbols() -> Result<usize>`
- [x] Implement `list_files() -> Result<Vec<PathBuf>>`

---

#### 2.3 🟢 References & Opens Storage
**Effort:** 2-3 hours  
**Dependencies:** 2.1  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Implement `insert_reference(&self, file: &Path, reference: &Reference)`
- [x] Implement `find_references(&self, name: &str) -> Result<Vec<Reference>>`
- [x] Implement `references_in_file(&self, file: &Path) -> Result<Vec<Reference>>`
- [x] Implement `insert_open(&self, file: &Path, module: &str, line: u32)`
- [x] Implement `opens_for_file(&self, file: &Path) -> Result<Vec<String>>`
- [x] Implement `clear_file(&self, file: &Path)` - clears symbols, refs, opens

---

### Milestone 3: Type Information Storage

#### 3.1 🟢 Type Member Operations
**Effort:** 2-3 hours  
**Dependencies:** 2.1  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Implement `insert_member(&self, member: &TypeMember) -> Result<i64>`
- [x] Implement `get_members(&self, type_name: &str) -> Result<Vec<TypeMember>>`
- [x] Implement `get_member(&self, type_name: &str, member_name: &str) -> Result<Option<TypeMember>>`
- [x] Implement `delete_type_members(&self, type_name: &str) -> Result<()>`
- [x] Implement batch insert: `insert_members(&self, members: &[TypeMember])`

---

#### 3.2 🟢 Type Signature in Symbols
**Effort:** 1-2 hours  
**Dependencies:** 2.1  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Schema has `type_signature` column
- [x] Schema has `source` column (syntactic/semantic)
- [x] Implement `get_symbol_type(&self, qualified: &str) -> Result<Option<String>>`
- [x] Implement `update_symbol_type(&self, qualified: &str, type_sig: &str)`

---

### Milestone 4: CLI Integration

#### 4.1 🟢 Update CLI for SQLite
**Effort:** 4-5 hours  
**Dependencies:** 2.1, 2.2, 2.3  
**Files:** `crates/fsharp-cli/src/main.rs`

**Completed:**
- [x] Update `cmd_build` to use SqliteIndex (stores to `.fsharp-index/index.db`)
- [x] Update `cmd_update` to use SqliteIndex
- [x] Update `cmd_def` to query SQLite
- [x] Update `cmd_refs` to query SQLite
- [x] Update `cmd_symbols` to query SQLite
- [x] Update `cmd_watch` to update SQLite
- [x] Update `cmd_type_info` to query SQLite
- [x] Add `load_code_index()` for spider compatibility (reads SQLite into CodeIndex)

**Notes:**
- Spider command still uses CodeIndex internally (loaded from SQLite)
- All 109 tests passing

---

### Milestone 5: F# Extraction Script Update

#### 5.1 🟢 Update Script for SQLite Output
**Effort:** 3-4 hours  
**Dependencies:** Schema defined (1.2)  
**Files:** `scripts/extract-types.fsx`

**Completed:**
- [x] Add Microsoft.Data.Sqlite reference
- [x] Implement `initDatabase` function with schema creation
- [x] Implement `insertSymbol` with type_signature
- [x] Implement `insertMember` for type members
- [x] Implement `updateSymbolType` to enhance existing symbols
- [x] Update metadata table with extraction timestamp
- [x] Handle transactions for atomic operations

---

#### 5.2 🟢 Merge Syntactic + Semantic Data
**Effort:** 2-3 hours  
**Dependencies:** 5.1  
**Files:** `scripts/extract-types.fsx`

**Completed:**
- [x] Update existing symbols by qualified name
- [x] Set `source = 'semantic'` for updated symbols
- [x] Insert new type members
- [x] Preserve syntactic-only symbols (only clear semantic data)
- [x] Clear distinction between sources

---

### Milestone 6: LSP Update

#### 6.1 🟢 Update LSP for SQLite
**Effort:** 3-4 hours  
**Dependencies:** 4.1  
**Files:** `crates/fsharp-lsp/src/main.rs`

**Completed:**
- [x] Load index from SQLite on startup (`load_index_from_sqlite`)
- [x] Fall back to building fresh if no SQLite index exists
- [x] Update `did_save` handler to write to both in-memory and SQLite
- [x] Add type signature display in hover (when available)
- [x] All existing features preserved (goto_definition, references, workspace_symbol)

---

#### 6.2 ⬜ Enhanced Hover with Types
**Effort:** 2-3 hours  
**Dependencies:** 6.1  
**Files:** `crates/fsharp-lsp/src/main.rs`

**Tasks:**
- [ ] Query type_signature on hover
- [ ] Format type signature in markdown
- [ ] Show parameter types for functions
- [ ] Show member type for properties/methods

**Acceptance Criteria:**
- Hover shows: `processUser : User -> Async<Result<Response, Error>>`
- Falls back to just name/location when no type info

---

### Milestone 7: Testing & Documentation

#### 7.1 🟢 SQLite Unit Tests
**Effort:** 3-4 hours  
**Dependencies:** All Milestone 2-3 tasks  
**Files:** `crates/fsharp-index/src/db.rs`

**Completed:**
- [x] Test schema creation
- [x] Test symbol CRUD operations
- [x] Test reference operations
- [x] Test opens operations
- [x] Test member operations
- [x] Test search with patterns
- [x] Test error handling (version mismatch, file not exists)
- [x] 23 SQLite-specific tests passing

---

#### 7.2 ⬜ Integration Tests
**Effort:** 4-5 hours  
**Dependencies:** 7.1  
**Files:** `tests/` directory

**Tasks:**
- [ ] End-to-end test: build → query → resolve
- [ ] Test with real F# project
- [ ] Test type extraction script
- [ ] Performance benchmarks vs old JSON approach

**Acceptance Criteria:**
- Full workflow tested
- No regressions from JSON approach

---

#### 7.3 ⬜ Update Documentation
**Effort:** 2-3 hours  
**Dependencies:** 7.2  
**Files:** `README.md`, `design/KNOWN_LIMITATIONS.md`, `scripts/README.md`

**Tasks:**
- [ ] Document SQLite storage format
- [ ] Update CLI usage examples
- [ ] Document `--extract-types` flag
- [ ] Document schema for custom tooling
- [ ] Update KNOWN_LIMITATIONS.md with new capabilities

**Acceptance Criteria:**
- Clear documentation for users
- Schema documented for tool authors

---

## Phase 2: Lightweight Rust Inference (Future)

> **Note:** Deferred until Phase 1 SQLite migration is complete.

### Milestone 8: Literal Type Inference

- [ ] 8.1 Infer types from literals (string, int, bool, list)
- [ ] 8.2 Extract type annotations from AST
- [ ] 8.3 Infer types from constructor calls

### Milestone 9: Real-Time Integration

- [ ] 9.1 Integrate inference with SQLite-backed resolution
- [ ] 9.2 Benchmark and optimize

---

## Summary

### Phase 1 Progress

| Milestone | Tasks | Status |
|-----------|-------|--------|
| 1. SQLite Foundation | 3 | 🟢 Complete |
| 2. Symbol Storage | 3 | 🟢 Complete |
| 3. Type Information | 2 | 🟢 Complete |
| 4. CLI Integration | 1 | 🟢 Complete |
| 5. F# Script Update | 2 | 🟢 Complete |
| 6. LSP Update | 1 | 🟢 Complete |
| 7. Testing & Documentation | 3 | 🟡 Partial (unit tests done) |

### Key Benefits of SQLite Migration

| Benefit | Description |
|---------|-------------|
| **O(log n) lookups** | Indexed queries vs O(n) HashMap build |
| **Low memory** | Query on-demand, don't load full index |
| **Incremental updates** | UPDATE single rows, no full rewrite |
| **Rich queries** | LIKE patterns, JOINs, aggregations |
| **Debugging** | Inspect with `sqlite3` CLI |

---

## Definition of Done for Phase 1

- [x] Milestone 1: SQLite Foundation complete
- [x] Milestone 2: Symbol Storage complete
- [x] Milestone 3: Type Information Storage complete  
- [x] Milestone 4: CLI Integration complete
- [x] Milestone 5: F# Script Update complete
- [x] Milestone 6: LSP Update complete
- [x] All existing tests passing (109 tests)
- [x] New SQLite tests passing (23 tests)
- [x] Hover shows type signatures (when available from type extraction)
- [ ] Integration tests with real F# projects
- [ ] Documentation updated
- [ ] Performance benchmarks