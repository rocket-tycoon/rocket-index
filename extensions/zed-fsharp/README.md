# F# Fast - Zed Extension

Lightning-fast F# language support for Zed. **100-800x faster** than traditional F# language servers.

## Why F# Fast?

| Metric | F# Fast | fsautocomplete | Improvement |
|--------|---------|----------------|-------------|
| Initialize | 5ms | 2,166ms | **483x faster** |
| Go to definition | 0.1ms | 30ms | **319x faster** |
| Completion | 2ms | 1,660ms | **889x faster** |
| Workspace symbol | 0.4ms | 6ms | **15x faster** |
| Memory usage | ~50 MB | 1-21 GB | **20-400x less** |

Pure Rust. No .NET runtime required. No project load delays. No unbounded memory growth.

## Features

- **Syntax highlighting** via tree-sitter-fsharp
- **Go to definition** for user-defined symbols
- **Find references** across your codebase
- **Workspace symbol search** (FTS5-powered)
- **Completion** (keywords + symbols + member completion)
- **Hover information** with type signatures
- **Code actions** (organize imports, add missing opens)
- **Rename** across files

## Installation

### For Users (Coming Soon)

Install from the Zed extension marketplace once released.

### For Local Development

1. **Build the LSP binary:**
   ```bash
   cd /path/to/rocket-index
   cargo build --release -p rocketindex-lsp
   ```

2. **Add to your shell profile** (`~/.zshrc` or `~/.bashrc`):
   ```bash
   export ROCKETINDEX_LSP_PATH="/path/to/rocket-index/target/release/rocketindex-lsp"
   ```

3. **Restart your terminal** (or run `source ~/.zshrc`)

4. **Uninstall any existing F# extension** in Zed (Extensions panel → F# → Uninstall)

5. **Install the dev extension in Zed:**
   - `Cmd+Shift+P` → `zed: install dev extension`
   - Select the `extensions/zed-fsharp` folder

6. **Restart Zed** to pick up the environment variable

The extension checks for `ROCKETINDEX_LSP_PATH` first, so you can iterate on the LSP without needing GitHub releases.

## Building the Index

Before using the LSP, build the symbol index for your F# project:

```bash
cd /path/to/your/fsharp-project
rocketindex build --root .
```

This creates a `.rocketindex/index.db` SQLite database with all symbols.

## Known Limitations

This is a lightweight, syntax-based language server. For full type-aware features, use [fsautocomplete](https://github.com/fsharp/FsAutoComplete).

## License

MIT
