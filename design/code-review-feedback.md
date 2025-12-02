# Code Review: fsharp-tools

**Date:** October 26, 2023
**Reviewer:** Language Server Expert
**Scope:** Architecture, Implementation, Performance, and LSP Best Practices

## Executive Summary

The implementation faithfully adheres to the "minimal" goals set out in the design document. It successfully establishes a modular architecture (`fsharp-index`, `fsharp-lsp`, `fsharp-cli`) and implements the core "Spider" logic for dependency graph traversal.

However, as a Language Server, it makes several significant trade-offs for simplicity that will impact User Experience (UX) and performance in medium-to-large codebases. The most critical issues are the lack of buffer synchronization (only works on saved files) and the "fake" end positions for symbols.

---

## 1. Architecture & Project Structure

**Strengths:**
*   **Modularity:** The split between `fsharp-index` (core logic) and `fsharp-lsp` (protocol adapter) is excellent. It allows the CLI tool to share the exact same logic as the server.
*   **Portability:** The `CodeIndex` correctly handles relative paths (`to_relative`/`to_absolute`), ensuring the index is portable across environments or when the project root changes.

**Weaknesses:**
*   **State Management:** The `Backend` struct holds the `CodeIndex` in an `Arc<RwLock<>>`. While correct for concurrency, the coarse-grained locking on the entire index might become a bottleneck if background indexing (on save) contends with read operations (go-to-definition).

## 2. Data Structures (`lib.rs`, `index.rs`)

**Critique:**
*   **`Location` Struct:**
    ```rust
    pub struct Location {
        pub file: PathBuf,
        pub line: u32,
        pub column: u32,
    }
    ```
    **Issue:** You are only storing the *start* position.
    **Impact:** In `fsharp-lsp/src/main.rs`, the `to_lsp_location` function hacks this by adding an arbitrary offset:
    ```rust
    end: Position {
        line: loc.line.saturating_sub(1),
        character: loc.column.saturating_sub(1) + 10, // Approximate end
    },
    ```
    This results in the editor highlighting a random 10-character range when jumping to a definition, which looks broken for short names (e.g., `x`) or long names (e.g., `AbstractFactorySingleton`).
    **Recommendation:** Store `end_line` and `end_column` in `Location`. Tree-sitter provides this information easily (`node.end_position()`).

*   **Symbol Storage:**
    The `definitions` map uses `String` (qualified name) as the key.
    **Issue:** F# allows function overloading (methods) and shadowing. If two symbols have the same qualified name, the last one indexed overwrites the previous one.
    **Recommendation:** Use a `MultiMap` or `Vec<Symbol>` as the value, or include the signature/arity in the key.

## 3. Parsing & Extraction (`parse.rs`)

**Strengths:**
*   **Tree-sitter Usage:** The recursion logic in `extract_recursive` is generally sound. It correctly handles nested modules and namespaces.
*   **Resilience:** The parser gracefully handles partial failures by logging warnings rather than crashing.

**Weaknesses:**
*   **Inefficient Re-parsing:**
    In `fsharp-lsp/src/main.rs`, `goto_definition` calls `get_symbol_at_position`, which:
    1.  Reads the file from disk (blocking I/O).
    2.  Re-initializes the Tree-sitter parser.
    3.  Re-parses the entire file.
    This happens on *every* click or hover (if hover were implemented).
    **Recommendation:** Cache the parse tree in memory, or at least read from the in-memory file buffer (see Section 5).

## 4. Resolution & Indexing (`resolve.rs`, `spider.rs`)

**Strengths:**
*   **Resolution Logic:** The 4-step resolution strategy (Qualified -> Same File -> Open -> Parent) is a solid implementation of F# scoping rules for a lightweight tool.
*   **Spider:** The BFS traversal in `spider.rs` is correctly implemented and useful for the "find references" or "impact analysis" goals.

**Weaknesses:**
*   **Performance in `resolve_in_parent_modules`:**
    ```rust
    fn resolve_in_parent_modules(...) {
        let file_symbols = self.symbols_in_file(from_file);
        for symbol in &file_symbols { ... }
    }
    ```
    This iterates *all* symbols in a file to deduce the current module context. For a file with 1,000 symbols, this is unnecessary work.
    **Recommendation:** Pass the context (current module) into the resolve function, or store a `file -> module` map in the index.

*   **Linear Search:**
    `CodeIndex::search` performs a linear scan (`filter`) over all definitions.
    **Impact:** On a project with 10k+ symbols, the "Workspace Symbol" (Cmd+T) feature will feel sluggish.
    **Recommendation:** For a v1.0, consider a trie or a simple inverted index for symbol names.

## 5. LSP Implementation (`main.rs`)

**Critical Issues:**

1.  **No Buffer Synchronization (`textDocument/didChange`):**
    The server ignores `didChange` events.
    ```rust
    async fn did_change(&self, _params: DidChangeTextDocumentParams) {
        // Mark file as dirty - we'll reindex on save
    }
    ```
    **Impact:** The server only knows about what is on *disk*. If a user types a new function and tries to jump to it before saving, it won't work. "Go to definition" will be calculated based on the file state at the last save, which might not match the current line numbers in the editor, causing jumps to the wrong location.
    **Recommendation:** Implement a `DocumentStore` that updates an in-memory string on `didChange`. Use this string for `get_symbol_at_position`.

2.  **Blocking Indexing:**
    `build_index` runs sequentially on the main thread (even if async, it's CPU bound parsing).
    **Impact:** For large projects, the "Language Server initialized" message might be delayed, and the server might be unresponsive during the initial crawl.

## 6. Recommendations for Next Steps

1.  **Fix `Location`:** Immediately update `Location` to include end positions. This is a low-effort, high-value fix for UX.
2.  **Implement `didChange`:** Store file contents in memory. This is a prerequisite for any "real" LSP features like completion or hover.
3.  **Optimize `get_symbol_at_position`:** Parse the in-memory string, not the file on disk.
4.  **Add `textDocument/hover`:** You already have the resolution logic. Showing the symbol's signature or doc comment (if extracted) would be a quick win.

**Verdict:** The codebase is a clean, well-structured "Proof of Concept". It meets the "Minimal" design goals but requires specific architectural changes (buffer management, precise locations) to graduate to a usable daily driver.