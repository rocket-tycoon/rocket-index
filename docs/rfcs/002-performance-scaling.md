# RFC 002: Performance Optimizations for Large Scale Repositories

- **Date**: 2025-12-26
- **Status**: Proposed
- **Area**: RocketIndex Core / CLI

## Summary

This RFC proposes significant architectural changes to `rocketindex-cli` and the SQLite schema to support indexing of extremely large repositories (50k+ files) without excessive memory usage or disk bloat. The primary focus is eliminating the "OOM Cliff" (Out Of Memory) during the indexing phase.

## Motivation

The current indexing implementation in `rocketindex-cli` uses a "gather-all-then-write" approach:
1.  Recursively find all source files.
2.  Parse *all* files in parallel using `rayon`, collecting *all* symbols, references, and opens into a single `Vec` in memory.
3.  Write the entire collection to SQLite in one batch.

While efficient for small-to-medium projects, this O(N) memory usage becomes catastrophic for large monorepos. A default Linux kernel checkout or a large enterprise repo can easily exceed available RAM, causing the process to be killed by the OS.

Additionally, the database schema stores the full file path string for every symbol, leading to significant disk space wastage (denormalization).

## Benchmark Targets

| Metric | Current | Target |
|--------|---------|--------|
| Linux kernel (70k files) indexing time | OOM/fails | < 60s |
| Peak memory usage (70k files) | unbounded | < 500 MB |
| Index size (70k files) | N/A | < 200 MB |
| Memory usage (10k files) | TBD | < 200 MB |

*Note: Baseline measurements needed before implementation.*

## Detailed Design

### 1. Chunked Indexing (Memory Fix)

**Priority**: High - Implement first (standalone, low risk)

**Change**: Modify the parallel parsing loop in `crates/rocketindex-cli/src/main.rs`.

Instead of `collect()`ing all results, we will:
1.  Use `par_bridge` or a custom chunking iterator to process files in batches (e.g., `BATCH_SIZE = 1000`).
2.  For each batch:
    *   Parse files in parallel.
    *   Accumulate results locally for the batch.
    *   Acquire a write lock on the SQLite index.
    *   Commit the batch to the database.
    *   Release memory.

**Implementation**:
```rust
for chunk in files.chunks(BATCH_SIZE) {
    let results: Vec<_> = chunk.par_iter().map(parse_file).collect();
    index.insert_batch(results)?;

    // Progress reporting
    processed += chunk.len();
    eprintln!("Indexed {}/{} files", processed, total);
}
```

**Impact**: Memory usage becomes O(BATCH_SIZE) instead of O(TOTAL_FILES). Upper bound on RAM usage is constant regardless of repo size.

**Transaction Strategy**:
- Each batch commits in its own transaction
- On failure, the database contains a partial index (all completed batches)
- The CLI should detect partial state and offer `--continue` or `--rebuild` options
- Consider storing indexing progress in a metadata table for recovery

**Progress Reporting**:
- Required for large repos - users need feedback during multi-minute operations
- Report: files indexed, files remaining, elapsed time
- Optional: `--quiet` flag to suppress progress output

### 2. Schema Normalization (Disk Space Fix)

**Priority**: Medium - Requires migration tooling

**Change**: Normalize file paths in `crates/rocketindex/src/db.rs`.

**Current Schema**:
```sql
TABLE symbols (
    ...
    file TEXT NOT NULL,  -- Repeated string
    ...
)
```

**Proposed Schema**:
```sql
TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
);

TABLE symbols (
    ...
    file_id INTEGER NOT NULL REFERENCES files(id),
    ...
)
```

**Migration Plan**:

1. **Schema Version**: Bump to version 5
2. **Migration SQL**:
```sql
-- Create new files table
CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
);

-- Populate from existing data
INSERT INTO files (path) SELECT DISTINCT file FROM symbols;

-- Add foreign key column
ALTER TABLE symbols ADD COLUMN file_id INTEGER;

-- Populate foreign keys
UPDATE symbols SET file_id = (SELECT id FROM files WHERE files.path = symbols.file);

-- Drop old column (requires table rebuild in SQLite)
-- ... standard SQLite column drop procedure ...
```

3. **Version Handling**:
   - New CLI refuses to read schema < 5 (prompt user to rebuild)
   - No backwards compatibility with old schema (clean break)

**File Lifecycle**:
- On full reindex: DROP all tables, rebuild from scratch
- Orphan cleanup not needed since we always rebuild completely
- Future: incremental indexing would require orphan detection (separate RFC)

**Query Impact**:
- All symbol queries now require JOIN to `files` table
- Benchmark hot paths (`find_definition`, `find_callers`) to verify negligible overhead
- SQLite INTEGER joins are fast, but measure to confirm

### 3. Deferred FTS Indexing (Speed Fix)

**Priority**: Low - Defer until FTS feature ships

**Status**: FTS is currently unused in the codebase. This optimization should only be implemented when full-text search functionality is added.

**Change**: Disable FTS triggers during bulk indexing.

Currently, `AFTER INSERT` triggers fire for every row inserted into `symbols`. For a fresh index of 100k symbols, this is 100k FTS updates.

**Optimization**:
1.  Drop `symbols_fts` and its triggers before starting the index process.
2.  Perform all Chunked Inserts.
3.  Re-create `symbols_fts` and populate it via `INSERT INTO symbols_fts SELECT ... FROM symbols`.

**Crash Recovery**:
- If process crashes after symbols inserted but before FTS rebuild:
  - Core index functionality works (symbols present)
  - FTS queries return empty/stale results
- Mitigation: Store `fts_valid` flag in metadata table
- On startup, check flag and rebuild FTS if invalid
- Add `rkt index --rebuild-fts` command for manual recovery

**Incremental Update Concern**:
- This approach works for full reindex only
- `rkt watch` adding single files cannot use deferred FTS (too expensive to rebuild)
- For watch mode: keep existing per-row FTS triggers
- Only use deferred FTS during `rkt index` full rebuild

## Alternatives Considered

*   **Streaming Channel**: Use a bounded channel (`crossbeam::channel`) where high-speed parsers (producers) feed a single database writer (consumer).
    *   *Pros*: Keeps DB writes strictly serial, potentially smoother CPU usage.
    *   *Cons*: More complex to implement correctly with `rayon`. Chunking is simpler and achieves similar memory safety.

*   **Vertical Partitioning**: Moving `doc` and `signature` to a separate table `symbol_details`.
    *   *Status*: Deferred. While beneficial for cache locality, it requires significantly more query rewriting. The OOM fix (Chunked Indexing) is higher priority.

*   **Incremental Indexing**: Track file hashes, only reparse changed files.
    *   *Status*: Deferred to separate RFC. Larger architectural change that affects all three proposals here.

## Testing Strategy

1. **Unit Tests**: Mock file system with configurable file counts
2. **Integration Tests**:
   - Create test corpus generator (synthetic files with realistic symbol density)
   - Test at 1k, 10k, 50k file boundaries
3. **Memory Validation**:
   - Use `/usr/bin/time -v` to capture peak RSS
   - CI job that fails if memory exceeds threshold
4. **Large Repo Testing**:
   - Clone linux kernel or chromium as manual validation
   - Not suitable for CI (too slow), but required before release

## Backwards Compatibility

- **Schema v4 → v5**: Breaking change, no migration path
- Old databases will be rejected with clear error message
- Users must rebuild index (`rkt index --force`)
- CLI version check: warn if index was built with newer CLI version

## Rollout Plan

1. **Phase 1**: Chunked indexing (this RFC, memory fix)
   - Feature-flag: none needed, always enabled
   - Schema version: unchanged (v4)

2. **Phase 2**: Schema normalization (this RFC, disk fix)
   - Schema version: bump to v5
   - Release notes: "Rebuild required after upgrade"

3. **Phase 3**: Deferred FTS (future, when FTS ships)
   - Gated behind FTS feature flag
   - Only activates during full reindex

## Unresolved Questions

*   **Optimal Batch Size**: 1000 seems safe, but benchmarking on varied hardware would confirm if larger batches (5000) offer better write throughput.

*   **Progress UI**: Simple stderr output vs. progress bar library (indicatif)?

*   **Partial Index UX**: How should CLI behave when detecting partial index from crashed run?

## Implementation Steps

### Phase 1: Chunked Indexing
1.  Add progress reporting infrastructure
2.  Refactor `cmd_index` to iterate in batches
3.  Add `--batch-size` flag for tuning (default 1000)
4.  Add memory benchmarks to CI

### Phase 2: Schema Normalization
5.  Implement `files` table and migration SQL
6.  Update `insert_symbol` to use file IDs
7.  Update all read queries to JOIN
8.  Bump schema version to 5
9.  Benchmark query performance regression

### Phase 3: Deferred FTS (when needed)
10. Add `fts_valid` metadata flag
11. Implement deferred FTS rebuild logic
12. Add `--rebuild-fts` CLI command
