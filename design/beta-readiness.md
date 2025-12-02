# fsharp-tools Beta Readiness Report

_Date: 2024-XX-XX_  
_Author: Release Management_

---

## Executive Summary

| Item | Status | Notes |
| --- | --- | --- |
| Test suite (`cargo test --all`) | ✅ Passing locally |
| Core feature scope (goto-def, references, hover, doc store) | ✅ Matches beta expectations per `TASKS.md` |
| Known architectural trade-offs | ✅ Documented in `design/KNOWN_LIMITATIONS.md` |
| Large-project validation (RocketSpec) | ⚠️ **Blocking** pending data |
| Performance metrics/observability | ⚠️ Need instrumentation on large workspaces |
| CLI/LSP UX polish | ⚠️ Minor but important gaps (`cmd_update`, spider backend) |
| Release packaging (version bump, notes, binaries) | ⚠️ Not started |

Until the highlighted blockers are closed—most critically the RocketSpec validation—we should not tag a beta release.

---

## Detailed Findings

### 1. RocketSpec Validation (Blocking)
- **Objective:** Prove `fsharp-tools` operates smoothly on a sizable F# codebase (RocketShip/RocketSpec at `/Users/alastair/work/rocket-tycoon/RocketSpec`).
- **Required evidence:**
  - Index build time, resulting SQLite DB size, peak RSS during `fsharp-index build`.
  - Runtime memory/CPU profile of `fsharp-lsp` while navigating RocketSpec in Zed (or via `--stdio`).
  - UX observations: definition accuracy, references latency, hover usefulness.
  - Any crashes, panics, or high-severity logs.
- **Execution plan (for the lead language server engineer):**
  1. `cd /Users/alastair/work/rocket-tycoon/RocketSpec`
  2. `cargo run -p fsharp-cli -- build --root . --output .fsharp-index/rocket_beta.db`
  3. Exercise standard workflows in Zed or via `fsharp-lsp --stdio`.
  4. Capture metrics + regressions in a short report back to this repo.

_No access to that workspace from here; this must be run on the engineer’s machine or CI._

### 2. Performance & Observability (Should-have)
- Add lightweight logging (guarded by `RUST_LOG`) around:
  - Index build duration and symbol counts.
  - LSP request latency (jump-to-def, references, hover).
- Optional: `--benchmark` CLI flag to run canned searches/definitions on an existing index.

### 3. CLI & Spider Polishing (High priority QoL)
- `fsharp-cli` `cmd_update` still brute-force re-indexes everything (see TODO around `#L568-597`). Even a simple mtime or checksum check would dramatically improve large-workspace flows.
- `cmd_spider` (around `#L705-735`) continues to rely on the in-memory `CodeIndex`; migrate it to `SqliteIndex` for consistency with build/update.

### 4. Release Packaging Steps
Once blockers above are cleared:

1. **Versioning**
   - Bump workspace + crate versions to `0.9.0-beta` (or chosen tag) in `Cargo.toml`.
2. **Artifacts**
   - Build release binaries (`cargo build --release -p fsharp-cli -p fsharp-lsp`).
   - Produce checksums and smoke-test binaries on macOS/Linux.
3. **Docs**
   - Draft beta release notes summarizing features, limitations, and install instructions.
   - Update `extensions/zed-fsharp` README/manifest to point at the beta artifacts.
4. **Announcement readiness**
   - Ensure Known Limitations are front-and-center for expectations management.
   - Collect RocketSpec metrics to cite in release notes.

---

## Action Plan

| Task | Owner | Deliverable |
| --- | --- | --- |
| RocketSpec validation run | Lead LSP engineer | Metrics + findings markdown, issues filed for any regressions |
| Performance logging toggle | Lead LSP engineer | `RUST_LOG`-gated timing/memory output + README instructions |
| Incremental `cmd_update` | Lead LSP engineer | Mtime-based reindexing, tests, docs |
| Spider migration to SQLite backend | Lead LSP engineer | Updated CLI implementation + verification |
| Release checklist & notes | Release manager | Draft notes, version bump PR, artifact verification |

---

## Release Decision Criteria

We can stamp “beta” once all of the following are true:

1. RocketSpec run shows acceptable performance (build time < benchmark TBD, no crashes, tolerable nav latency).
2. Observability/logging in place to measure regressions going forward.
3. Incremental update + spider alignment merged (or explicitly deferred with justification).
4. Release packaging artifacts complete and documented.

Until then, continue treating `fsharp-tools` as “internal daily-driver” quality rather than a public beta.