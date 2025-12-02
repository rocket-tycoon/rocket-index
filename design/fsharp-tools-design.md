# fsharp-tools: A Minimal F# Language Server for Zed

**Status**: Design  
**Author**: Alastair  
**Date**: December 2025

## Executive Summary

Build a lightweight, memory-stable F# language server optimized for AI-assisted coding workflows. The server provides codebase indexing and navigation without the unbounded memory growth that plagues fsautocomplete. Written entirely in Rust using tree-sitter for parsing—no .NET runtime required.

### Non-Goals

- Feature parity with fsautocomplete
- Type-aware completions or hover information
- Refactoring tools (AI agents handle this)
- MCP integration (causes context bloat)

### Goals

- Bounded, predictable memory usage (<50MB for large codebases)
- Fast codebase indexing and navigation
- Go-to-definition across files
- Dependency graph traversal ("spider" from entry point)
- Clean CLI for AI agent tooling
- Zed extension with syntax highlighting

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Zed Editor                               │
│  ┌─────────────────┐  ┌──────────────────────────────────────┐  │
│  │ tree-sitter     │  │           LSP Client                 │  │
│  │ (highlighting)  │  │  (definition, workspace/symbol)      │  │
│  └─────────────────┘  └──────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
         │                              │
         │ grammar queries              │ JSON-RPC over stdio
         ▼                              ▼
┌─────────────────┐           ┌─────────────────────────────────┐
│ tree-sitter-    │           │         fsharp-lsp              │
│ fsharp          │           │  (thin wrapper over fsharp-index)│
│ (compiled in)   │           └─────────────────────────────────┘
└─────────────────┘                          │
                                             │ library calls
                                             ▼
                              ┌─────────────────────────────────┐
                              │        fsharp-index             │
                              │  ┌───────────┐ ┌─────────────┐  │
                              │  │  parse.rs │ │  index.rs   │  │
                              │  │ (extract  │ │ (storage +  │  │
                              │  │  symbols) │ │  queries)   │  │
                              │  └───────────┘ └─────────────┘  │
                              │  ┌───────────┐ ┌─────────────┐  │
                              │  │resolve.rs │ │  spider.rs  │  │
                              │  │ (name     │ │ (graph      │  │
                              │  │  lookup)  │ │  traversal) │  │
                              │  └───────────┘ └─────────────┘  │
                              └─────────────────────────────────┘
                                             │
                                             ▼
                              ┌─────────────────────────────────┐
                              │         fsharp-cli              │
                              │   $ fsharp-index def "Foo.bar"  │
                              │   $ fsharp-index spider "main"  │
                              └─────────────────────────────────┘
```

---

## Project Structure

```
fsharp-tools/
├── Cargo.toml                      # Workspace definition
├── README.md
├── LICENSE                         # MIT
│
├── crates/
│   ├── fsharp-index/               # Core library
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              # Public API
│   │       ├── parse.rs            # tree-sitter symbol extraction
│   │       ├── index.rs            # Symbol storage and queries
│   │       ├── resolve.rs          # Name resolution with scope rules
│   │       ├── spider.rs           # Dependency graph traversal
│   │       └── watch.rs            # File system watcher for incremental updates
│   │
│   ├── fsharp-cli/                 # Command-line tool
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs
│   │
│   └── fsharp-lsp/                 # Language server
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
│
└── extensions/
    └── zed-fsharp/                 # Zed extension
        ├── Cargo.toml
        ├── extension.toml
        ├── src/
        │   └── lib.rs
        └── languages/
            └── fsharp/
                ├── config.toml
                ├── highlights.scm
                ├── brackets.scm
                ├── folds.scm
                ├── indents.scm
                └── outline.scm
```

---

## Core Data Structures

### Symbol

```rust
// crates/fsharp-index/src/lib.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Location {
    pub file: PathBuf,
    pub line: u32,      // 1-indexed
    pub column: u32,    // 1-indexed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Module,
    Function,
    Value,
    Type,
    Record,
    Union,
    Interface,
    Class,
    Member,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,           // Short name: "processPayment"
    pub qualified: String,      // Full path: "MyApp.Services.PaymentService.processPayment"
    pub kind: SymbolKind,
    pub location: Location,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Internal,
    Private,
}
```

### Index

```rust
// crates/fsharp-index/src/index.rs

pub struct CodeIndex {
    /// Symbol name -> definition location
    /// Key is qualified name: "MyApp.Services.PaymentService.processPayment"
    definitions: HashMap<String, Symbol>,
    
    /// File -> symbols defined in that file
    file_symbols: HashMap<PathBuf, Vec<String>>,
    
    /// File -> symbol references (identifiers used, not defined)
    file_references: HashMap<PathBuf, Vec<Reference>>,
    
    /// Module/namespace -> files that define symbols in it
    module_files: HashMap<String, Vec<PathBuf>>,
    
    /// File -> parsed opens/imports
    file_opens: HashMap<PathBuf, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub name: String,       // The identifier as written: "List.map" or "processPayment"
    pub location: Location, // Where it appears
}
```

### Index Persistence

Store index as JSON in `.fsharp-index/` at workspace root:

```
.fsharp-index/
├── index.json          # Main index file
├── version             # Schema version for migration
└── files/              # Per-file cache for incremental updates
    ├── src_Program.fs.json
    ├── src_Services_Payment.fs.json
    └── ...
```

This allows:
- Fast startup (load from disk, not reparse)
- Incremental updates (only reparse changed files)
- Git-ignorable (add to .gitignore)

---

## Algorithms

### Symbol Extraction (parse.rs)

Use tree-sitter-fsharp to walk the CST and extract definitions:

```rust
pub fn extract_symbols(path: &Path, source: &str) -> ParseResult {
    let mut parser = Parser::new();
    parser.set_language(tree_sitter_fsharp::language())?;
    
    let tree = parser.parse(source, None)?;
    let mut symbols = Vec::new();
    let mut references = Vec::new();
    let mut opens = Vec::new();
    
    let mut cursor = tree.walk();
    extract_recursive(&mut cursor, source.as_bytes(), path, &mut symbols, &mut references, &mut opens);
    
    ParseResult { symbols, references, opens }
}

fn extract_recursive(
    cursor: &mut TreeCursor,
    source: &[u8],
    path: &Path,
    symbols: &mut Vec<Symbol>,
    references: &mut Vec<Reference>,
    opens: &mut Vec<String>,
) {
    loop {
        let node = cursor.node();
        
        match node.kind() {
            // Module declaration: module MyApp.Services
            "module_defn" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    symbols.push(Symbol {
                        name: last_component(node_text(name_node, source)),
                        qualified: node_text(name_node, source).to_string(),
                        kind: SymbolKind::Module,
                        location: node_location(name_node, path),
                        visibility: Visibility::Public,
                    });
                }
            }
            
            // Let binding: let processPayment x = ...
            "function_or_value_defn" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    let name = extract_binding_name(pattern, source);
                    let visibility = if has_private_modifier(node) {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
                    symbols.push(Symbol {
                        name: name.clone(),
                        qualified: name, // Will be qualified later with module context
                        kind: infer_kind(node),
                        location: node_location(pattern, path),
                        visibility,
                    });
                }
            }
            
            // Type definition: type PaymentResult = ...
            "type_definition" => {
                // Extract type name and kind (record, union, class, etc.)
            }
            
            // Open statement: open System.Collections.Generic
            "open_statement" => {
                if let Some(module_node) = node.child_by_field_name("module") {
                    opens.push(node_text(module_node, source).to_string());
                }
            }
            
            // Any identifier usage is a potential reference
            "long_identifier" | "identifier" => {
                // Skip if this is part of a definition (already handled above)
                if !is_definition_context(node) {
                    references.push(Reference {
                        name: node_text(node, source).to_string(),
                        location: node_location(node, path),
                    });
                }
            }
            
            _ => {}
        }
        
        // Recurse into children
        if cursor.goto_first_child() {
            extract_recursive(cursor, source, path, symbols, references, opens);
            cursor.goto_parent();
        }
        
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}
```

### Name Resolution (resolve.rs)

Resolve an identifier to its definition location:

```rust
impl CodeIndex {
    pub fn resolve(&self, name: &str, from_file: &Path) -> Option<&Symbol> {
        // 1. Try fully qualified lookup first
        if let Some(sym) = self.definitions.get(name) {
            return Some(sym);
        }
        
        // 2. Get opens for this file
        let opens = self.file_opens.get(from_file)?;
        
        // 3. Try each opened module as prefix
        for open in opens {
            let qualified = format!("{}.{}", open, name);
            if let Some(sym) = self.definitions.get(&qualified) {
                return Some(sym);
            }
        }
        
        // 4. Try same-file definitions (handles local scope)
        if let Some(file_syms) = self.file_symbols.get(from_file) {
            for sym_name in file_syms {
                if sym_name.ends_with(&format!(".{}", name)) || sym_name == name {
                    return self.definitions.get(sym_name);
                }
            }
        }
        
        // 5. Not found (probably BCL or external package)
        None
    }
}
```

### Spider (spider.rs)

Traverse the dependency graph from an entry point:

```rust
pub fn spider(
    index: &CodeIndex,
    start: &str,
    max_depth: u32,
) -> Vec<Location> {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut result: Vec<Location> = Vec::new();
    let mut queue: VecDeque<(Location, u32)> = VecDeque::new();
    
    // Find starting symbol
    let start_sym = match index.resolve_global(start) {
        Some(s) => s,
        None => return result,
    };
    
    queue.push_back((start_sym.location.clone(), 0));
    
    while let Some((loc, depth)) = queue.pop_front() {
        // Skip if already visited or too deep
        if !visited.insert(loc.file.clone()) {
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        
        result.push(loc.clone());
        
        // Get all references from this file
        if let Some(refs) = index.file_references.get(&loc.file) {
            for reference in refs {
                // Try to resolve each reference
                if let Some(sym) = index.resolve(&reference.name, &loc.file) {
                    if !visited.contains(&sym.location.file) {
                        queue.push_back((sym.location.clone(), depth + 1));
                    }
                }
            }
        }
    }
    
    result
}
```

---

## CLI Interface

```bash
# Build/rebuild the index for current directory
$ fsharp-index build
Indexed 47 files, 523 symbols in 0.34s

# Incremental update (only changed files)
$ fsharp-index update
Updated 2 files

# Find definition
$ fsharp-index def "PaymentService.processPayment"
src/Services/Payment.fs:42:5

# Find definition with context (shows the line)
$ fsharp-index def "PaymentService.processPayment" --context
src/Services/Payment.fs:42:5
    let processPayment (request: PaymentRequest) =

# List references in a file
$ fsharp-index refs src/Services/Payment.fs
StripeClient.createCharge    src/Clients/Stripe.fs:18:5
AuditLog.record              src/Logging/Audit.fs:7:5
Config.getStripeKey          src/Config.fs:12:5
Result.bind                  <external>
Async.bind                   <external>

# Spider from entry point
$ fsharp-index spider "Program.main" --depth 5
src/Program.fs:10:5          Program.main
src/Startup.fs:25:5          Startup.configureApp
src/Handlers.fs:15:5         Handlers.paymentHandler
src/Services/Payment.fs:42:5 PaymentService.processPayment
src/Clients/Stripe.fs:18:5   StripeClient.createCharge

# List all symbols matching pattern
$ fsharp-index symbols "Payment*"
PaymentService               src/Services/Payment.fs:10:1       Module
PaymentService.processPayment src/Services/Payment.fs:42:5      Function
PaymentRequest               src/Types/Payment.fs:5:1          Record
PaymentResult                src/Types/Payment.fs:15:1         Union

# Output as JSON for tooling
$ fsharp-index def "PaymentService.processPayment" --json
{"file":"src/Services/Payment.fs","line":42,"column":5}

# Watch mode for continuous indexing
$ fsharp-index watch
Watching for changes... (Ctrl+C to stop)
```

### Exit Codes

- `0`: Success
- `1`: Symbol not found
- `2`: Index doesn't exist (run `build` first)
- `3`: Parse error in source files
- `4`: Invalid arguments

---

## LSP Server

Minimal implementation exposing index via LSP:

### Supported Methods

| Method | Description |
|--------|-------------|
| `initialize` | Declare capabilities |
| `initialized` | Trigger initial index build |
| `shutdown` / `exit` | Clean shutdown |
| `textDocument/didOpen` | Track open files |
| `textDocument/didChange` | Mark file dirty |
| `textDocument/didSave` | Trigger reindex of file |
| `textDocument/didClose` | Stop tracking file |
| `textDocument/definition` | Resolve symbol at position |
| `workspace/symbol` | Search symbols by name |

### Server Implementation

```rust
// crates/fsharp-lsp/src/main.rs

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use fsharp_index::{CodeIndex, Location};

struct Backend {
    client: Client,
    index: RwLock<CodeIndex>,
    workspace_root: RwLock<Option<PathBuf>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Store workspace root
        if let Some(root) = params.root_uri {
            *self.workspace_root.write().await = Some(root.to_file_path().unwrap());
        }
        
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // Build initial index
        if let Some(root) = self.workspace_root.read().await.as_ref() {
            let mut index = self.index.write().await;
            if let Err(e) = index.build(root) {
                self.client.log_message(MessageType::ERROR, format!("Index build failed: {}", e)).await;
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        
        let file = uri.to_file_path().map_err(|_| tower_lsp::jsonrpc::Error::invalid_params("Invalid URI"))?;
        
        // Get word at position (simplified - real impl would use tree-sitter)
        let word = self.get_word_at_position(&file, pos).await?;
        
        let index = self.index.read().await;
        if let Some(sym) = index.resolve(&word, &file) {
            let loc = Location {
                uri: Url::from_file_path(&sym.location.file).unwrap(),
                range: Range {
                    start: Position { line: sym.location.line - 1, character: sym.location.column - 1 },
                    end: Position { line: sym.location.line - 1, character: sym.location.column - 1 + sym.name.len() as u32 },
                },
            };
            Ok(Some(GotoDefinitionResponse::Scalar(loc)))
        } else {
            Ok(None)
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = &params.query;
        let index = self.index.read().await;
        
        let matches: Vec<SymbolInformation> = index
            .search(query)
            .into_iter()
            .take(50) // Limit results
            .map(|sym| SymbolInformation {
                name: sym.name.clone(),
                kind: to_lsp_symbol_kind(sym.kind),
                location: to_lsp_location(&sym.location),
                container_name: Some(sym.qualified.rsplit_once('.').map(|(c, _)| c.to_string()).unwrap_or_default()),
                tags: None,
                deprecated: None,
            })
            .collect();
        
        Ok(Some(matches))
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let file = params.text_document.uri.to_file_path().unwrap();
        let mut index = self.index.write().await;
        if let Err(e) = index.update_file(&file) {
            self.client.log_message(MessageType::ERROR, format!("Reindex failed: {}", e)).await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        index: RwLock::new(CodeIndex::new()),
        workspace_root: RwLock::new(None),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
```

---

## Zed Extension

### extension.toml

```toml
id = "fsharp"
name = "F#"
description = "F# language support with fast, stable indexing"
version = "0.1.0"
schema_version = 1
authors = ["Your Name <you@example.com>"]
repository = "https://github.com/yourname/fsharp-tools"

[language_servers.fsharp-lsp]
name = "F# Language Server"
languages = ["F#"]

[grammars.fsharp]
repository = "https://github.com/ionide/tree-sitter-fsharp"
rev = "main"
path = "fsharp"
```

### src/lib.rs

```rust
use std::fs;
use zed_extension_api::{self as zed, Result};

struct FSharpExtension {
    cached_binary_path: Option<String>,
}

impl zed::Extension for FSharpExtension {
    fn new() -> Self {
        Self { cached_binary_path: None }
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let binary_path = self.ensure_binary()?;
        
        Ok(zed::Command {
            command: binary_path,
            args: vec![],
            env: worktree.shell_env(),
        })
    }
}

impl FSharpExtension {
    fn ensure_binary(&mut self) -> Result<String> {
        // Return cached path if available
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok() {
                return Ok(path.clone());
            }
        }

        // Determine platform
        let (platform, arch) = zed::current_platform();
        let platform_str = match platform {
            zed::Os::Mac => "apple-darwin",
            zed::Os::Linux => "unknown-linux-gnu",
            zed::Os::Windows => "pc-windows-msvc",
        };
        let arch_str = match arch {
            zed::Architecture::Aarch64 => "aarch64",
            zed::Architecture::X86 => "x86",
            zed::Architecture::X8664 => "x86_64",
        };

        // Get latest release
        let release = zed::latest_github_release(
            "yourname/fsharp-tools",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset_name = format!("fsharp-lsp-{}-{}.tar.gz", arch_str, platform_str);
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| format!("No binary for {}-{}", arch_str, platform_str))?;

        let version_dir = format!("fsharp-lsp-{}", release.version);
        let binary_path = format!("{}/fsharp-lsp", version_dir);

        // Download if not present
        if fs::metadata(&binary_path).is_err() {
            zed::download_file(
                &asset.download_url,
                &version_dir,
                zed::DownloadedFileType::GzipTar,
            )?;
            zed::make_file_executable(&binary_path)?;
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

zed::register_extension!(FSharpExtension);
```

### languages/fsharp/config.toml

```toml
name = "F#"
grammar = "fsharp"
path_suffixes = ["fs", "fsi", "fsx"]
line_comments = ["//"]
block_comment = ["(*", "*)"]
autoclose_before = ";:.,=}])>"
brackets = [
    { start = "{", end = "}", close = true, newline = true },
    { start = "[", end = "]", close = true, newline = true },
    { start = "(", end = ")", close = true, newline = true },
    { start = "[|", end = "|]", close = true, newline = true },
    { start = "[<", end = ">]", close = true, newline = false },
]
word_characters = ["_", "'"]
tab_size = 4
```

### languages/fsharp/highlights.scm

```scheme
; Keywords
[
  "let" "rec" "and" "in"
  "if" "then" "else" "elif"
  "match" "with" "when"
  "for" "to" "downto" "do" "done" "while"
  "try" "finally" "raise"
  "fun" "function"
  "type" "of" "as"
  "module" "namespace" "open"
  "val" "mutable" "inline" "static" "member" "override" "abstract" "default"
  "public" "private" "internal"
  "new" "inherit" "interface" "class" "struct" "enum" "delegate"
  "true" "false" "null"
  "async" "lazy" "yield" "yield!" "return" "return!"
  "use" "use!"
  "begin" "end"
] @keyword

; Operators
[
  "|>" "<|" ">>" "<<" 
  "||" "&&" 
  "=" "<>" "<" ">" "<=" ">="
  "+" "-" "*" "/" "%" "**"
  "::" "@" "^"
  "|" "&" "~~~" ">>>" "<<<"
  "->" "<-" ":>" ":?>" ":?"
] @operator

; Punctuation
["(" ")" "[" "]" "{" "}" "[|" "|]" "[<" ">]"] @punctuation.bracket
["," ";" "::" ":" "|" "."] @punctuation.delimiter

; Types
(type_name) @type
(type_argument) @type

; Functions
(function_or_value_defn 
  (function_declaration_left (identifier) @function))
(application_expression
  (long_identifier_or_op (long_identifier) @function.call))

; Parameters
(argument_patterns (long_identifier (identifier) @variable.parameter))

; Variables
(identifier) @variable
(long_identifier) @variable

; Literals
(string) @string
(verbatim_string) @string
(triple_quoted_string) @string
(char) @string
(int) @number
(float) @number
(bool) @constant.builtin

; Comments
(comment) @comment
(block_comment) @comment
(xml_doc) @comment.documentation

; Attributes
(attribute) @attribute

; Module/Namespace
(namespace (long_identifier) @namespace)
(module_defn (long_identifier) @namespace)
(open_statement (long_identifier) @namespace)
```

---

## Testing Strategy

### Unit Tests

```rust
// crates/fsharp-index/src/parse.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_let_binding() {
        let source = r#"
module Test

let add x y = x + y
"#;
        let result = extract_symbols(Path::new("test.fs"), source);
        
        assert_eq!(result.symbols.len(), 2); // module + function
        assert_eq!(result.symbols[1].name, "add");
        assert_eq!(result.symbols[1].kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_type_definition() {
        let source = r#"
type Person = { Name: string; Age: int }
"#;
        let result = extract_symbols(Path::new("test.fs"), source);
        
        assert_eq!(result.symbols[0].name, "Person");
        assert_eq!(result.symbols[0].kind, SymbolKind::Record);
    }

    #[test]
    fn extracts_references() {
        let source = r#"
let result = List.map (fun x -> x + 1) myList
"#;
        let result = extract_symbols(Path::new("test.fs"), source);
        
        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"List.map"));
        assert!(ref_names.contains(&"myList"));
    }

    #[test]
    fn resolves_with_open() {
        let mut index = CodeIndex::new();
        index.add_file(Path::new("src/Utils.fs"), r#"
module MyApp.Utils
let helper x = x
"#);
        index.add_file(Path::new("src/Main.fs"), r#"
module MyApp.Main
open MyApp.Utils
let run () = helper 1
"#);
        
        let resolved = index.resolve("helper", Path::new("src/Main.fs"));
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().location.file, Path::new("src/Utils.fs"));
    }
}
```

### Integration Tests

Create a test fixture directory with real F# projects:

```
tests/fixtures/
├── simple/
│   ├── Program.fs
│   └── Lib.fs
├── multi-project/
│   ├── App/
│   │   ├── App.fsproj
│   │   └── Program.fs
│   └── Lib/
│       ├── Lib.fsproj
│       └── Library.fs
└── edge-cases/
    ├── shadowing.fs
    ├── nested-modules.fs
    └── computation-expressions.fs
```

```rust
// crates/fsharp-index/tests/integration.rs

#[test]
fn indexes_real_project() {
    let fixture = Path::new("tests/fixtures/simple");
    let index = CodeIndex::build(fixture).unwrap();
    
    // Should find entry point
    let main = index.resolve_global("Program.main");
    assert!(main.is_some());
    
    // Spider should find dependencies
    let deps = spider(&index, "Program.main", 3);
    assert!(deps.len() >= 2);
}
```

### CLI Tests

```bash
#!/bin/bash
# tests/cli_test.sh

set -e

cd tests/fixtures/simple

# Build index
fsharp-index build
[ -d ".fsharp-index" ] || exit 1

# Test def lookup
result=$(fsharp-index def "Program.main")
[[ "$result" == *"Program.fs"* ]] || exit 1

# Test spider
result=$(fsharp-index spider "Program.main" --depth 3)
[[ "$result" == *"Lib.fs"* ]] || exit 1

echo "CLI tests passed"
```

---

## Local Development with Zed

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install tree-sitter CLI (for grammar development)
cargo install tree-sitter-cli

# Clone the project
git clone https://github.com/yourname/fsharp-tools
cd fsharp-tools
```

### Building

```bash
# Build all crates
cargo build --release

# Build just the LSP server
cargo build --release -p fsharp-lsp

# Build the Zed extension
cd extensions/zed-fsharp
cargo build --release --target wasm32-wasi
```

### Testing the Extension Locally

1. **Install the extension in dev mode:**

   ```bash
   # From the extensions/zed-fsharp directory
   # Copy to Zed's extension dev directory
   mkdir -p ~/.local/share/zed/extensions/work/fsharp
   cp -r . ~/.local/share/zed/extensions/work/fsharp/
   ```

2. **Or use Zed's extension development workflow:**

   Open Zed, run command: `zed: install dev extension`
   
   Navigate to `extensions/zed-fsharp` directory.

3. **Configure Zed to use local binary (during development):**

   Edit `~/.config/zed/settings.json`:
   
   ```json
   {
     "lsp": {
       "fsharp-lsp": {
         "binary": {
           "path": "/path/to/fsharp-tools/target/release/fsharp-lsp"
         }
       }
     }
   }
   ```

4. **View LSP logs:**

   In Zed, run command: `zed: open log`
   
   Filter for "fsharp" to see language server communication.

5. **Test features:**
   
   - Open an F# file
   - Verify syntax highlighting works
   - Ctrl+click on an identifier to test go-to-definition
   - Cmd+T / Ctrl+T to test workspace symbol search

### Debugging

```bash
# Run LSP server manually with logging
RUST_LOG=debug ./target/release/fsharp-lsp 2>lsp.log

# In another terminal, send LSP messages manually
cat <<EOF | ./target/release/fsharp-lsp
Content-Length: 123

{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":"file:///path/to/project"}}
EOF
```

---

## Deployment

### GitHub Actions CI

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all

  build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
          - os: macos-latest
            target: x86_64-apple-darwin
          - os: macos-latest
            target: aarch64-apple-darwin
          - os: windows-latest
            target: x86_64-pc-windows-msvc
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - run: cargo build --release --target ${{ matrix.target }} -p fsharp-lsp
      - uses: actions/upload-artifact@v4
        with:
          name: fsharp-lsp-${{ matrix.target }}
          path: target/${{ matrix.target }}/release/fsharp-lsp*
```

### Release Workflow

```yaml
# .github/workflows/release.yml
name: Release

on:
  push:
    tags: ['v*']

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      # Download all platform builds from CI
      - uses: actions/download-artifact@v4
      
      # Create tarballs
      - name: Package binaries
        run: |
          for dir in fsharp-lsp-*; do
            target=${dir#fsharp-lsp-}
            tar -czvf "fsharp-lsp-${target}.tar.gz" -C "$dir" .
          done
      
      # Create GitHub release
      - uses: softprops/action-gh-release@v1
        with:
          files: fsharp-lsp-*.tar.gz
```

### Submitting to Zed Extensions

1. **Publish the language server releases first** (as described above)

2. **Fork zed-industries/extensions**

3. **Add your extension:**

   ```bash
   cd extensions
   git submodule add https://github.com/yourname/fsharp-tools extensions/fsharp
   ```

4. **Add to extensions.toml:**

   ```toml
   [fsharp]
   submodule = "extensions/fsharp"
   path = "extensions/zed-fsharp"
   ```

5. **Submit PR to zed-industries/extensions**

   The Zed team will review for:
   - Security (no malicious code)
   - Quality (works as advertised)
   - Maintenance (you'll maintain it)

6. **After merge**, the extension appears in Zed's extension marketplace.

---

## Milestones

### v0.1.0 - Syntax Only
- [ ] Zed extension with tree-sitter-fsharp
- [ ] Syntax highlighting (highlights.scm)
- [ ] Bracket matching, folding, indentation
- [ ] No LSP, just grammar

### v0.2.0 - Local Navigation
- [ ] fsharp-index library with parsing
- [ ] CLI: `build`, `def`, `refs`
- [ ] Index persistence
- [ ] File watcher for incremental updates

### v0.3.0 - LSP Integration
- [ ] fsharp-lsp server
- [ ] textDocument/definition
- [ ] workspace/symbol
- [ ] Zed extension downloads binary

### v0.4.0 - Spider
- [ ] CLI: `spider` command
- [ ] Cross-file dependency graph
- [ ] Depth limiting

### v1.0.0 - Production Ready
- [ ] Published to Zed extensions
- [ ] All platforms (macOS, Linux, Windows)
- [ ] ARM64 + x86_64
- [ ] Documentation
- [ ] <50MB memory on large codebases

---

## Open Questions

1. **Should we support .fsproj parsing?** Would allow understanding project structure and references between projects. Adds complexity but improves accuracy.

2. **How to handle signature files (.fsi)?** They define public API. Should we prioritize them for symbol lookup?

3. **External dependencies?** Currently we return `<external>` for BCL/NuGet references. Could we index NuGet packages too?

4. **Diagnostics?** Parsing `dotnet build` stderr would give real errors. Worth including?

---

## References

- [tree-sitter-fsharp](https://github.com/ionide/tree-sitter-fsharp)
- [Zed Extension Development](https://zed.dev/docs/extensions/developing-extensions)
- [tower-lsp](https://github.com/ebkalderon/tower-lsp)
- [LSP Specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/)
- [rust-analyzer Architecture](https://rust-analyzer.github.io/blog/2023/07/24/durable-incrementality.html)
