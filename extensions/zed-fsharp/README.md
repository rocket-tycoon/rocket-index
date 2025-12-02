# F# Extension for Zed

Fast, lightweight F# language support for the Zed editor.

## Features

- **Syntax highlighting** via tree-sitter-fsharp
- **Go to definition** for user-defined symbols
- **Find references** across your codebase
- **Workspace symbol search**
- **Hover information** with type signatures (when available)

## Installation

### For Users (Coming Soon)

Install from the Zed extension marketplace once released.

### For Local Development

1. **Build the LSP binary:**
   ```bash
   cd /path/to/fsharp-tools
   cargo build --release -p fsharp-lsp
   ```

2. **Add to your shell profile** (`~/.zshrc` or `~/.bashrc`):
   ```bash
   export FSHARP_LSP_PATH="/path/to/fsharp-tools/target/release/fsharp-lsp"
   ```

3. **Restart your terminal** (or run `source ~/.zshrc`)

4. **Uninstall any existing F# extension** in Zed (Extensions panel → F# → Uninstall)

5. **Install the dev extension in Zed:**
   - `Cmd+Shift+P` → `zed: install dev extension`
   - Select the `extensions/zed-fsharp` folder

6. **Restart Zed** to pick up the environment variable

The extension checks for `FSHARP_LSP_PATH` first, so you can iterate on the LSP without needing GitHub releases.

## Building the Index

Before using the LSP, build the symbol index for your F# project:

```bash
cd /path/to/your/fsharp-project
fsharp-index build --root .
```

This creates a `.fsharp-index/index.db` SQLite database with all symbols.

## Known Limitations

This is a lightweight, syntax-based language server. For full type-aware features, use [fsautocomplete](https://github.com/fsharp/FsAutoComplete).

See [KNOWN_LIMITATIONS.md](../../design/KNOWN_LIMITATIONS.md) for details.

## License

MIT
