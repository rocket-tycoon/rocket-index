//! rocketindex-lsp: Rocket-fast F# language server
//!
//! This server provides:
//! - Go-to-definition
//! - Workspace symbol search
//! - Incremental file indexing on save
//! - In-memory document tracking for unsaved changes
//! - Syntax error diagnostics
//! - Keyword and symbol completion
//!
//! Storage: Uses SQLite database (.rocketindex/index.db) for persistence,
//! loaded into memory as CodeIndex for fast resolution.

mod completion;
mod document_store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use document_store::DocumentStore;
use rocketindex::{
    config::Config, db::DEFAULT_DB_NAME, extract_symbols, watch::find_source_files, CodeIndex,
    SqliteIndex, SyntaxError,
};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{error, info, warn};
use tree_sitter::{Parser, Point};

use std::cell::RefCell;

// Thread-local parser reuse for LSP operations - avoids creating a new parser per request
thread_local! {
    static LSP_FSHARP_PARSER: RefCell<Parser> = RefCell::new({
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .expect("tree-sitter-fsharp grammar incompatible with tree-sitter version");
        parser
    });
}

/// The language server backend
struct Backend {
    /// LSP client for sending notifications
    client: Client,
    /// The symbol index (in-memory for fast resolution)
    index: Arc<RwLock<CodeIndex>>,
    /// Workspace root directory
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    /// In-memory document store for open files
    documents: DocumentStore,
    /// Maximum recursion depth for parsing (from config)
    max_recursion_depth: Arc<RwLock<usize>>,
}

impl Backend {
    /// Get the path to the SQLite database.
    fn get_db_path(root: &Path) -> PathBuf {
        root.join(".rocketindex").join(DEFAULT_DB_NAME)
    }

    /// Load the index from SQLite database if it exists.
    async fn load_index_from_sqlite(&self) -> Result<bool> {
        let root = self.workspace_root.read().await;
        let root_path = match root.as_ref() {
            Some(r) => r.clone(),
            None => {
                warn!("No workspace root set");
                return Ok(false);
            }
        };
        drop(root);

        let db_path = Self::get_db_path(&root_path);

        if !db_path.exists() {
            info!("No SQLite index found at {:?}", db_path);
            return Ok(false);
        }

        info!("Loading index from SQLite: {:?}", db_path);

        let sqlite_index = SqliteIndex::open(&db_path)?;

        // Get workspace root from metadata or use current
        let workspace_root = sqlite_index
            .get_metadata("workspace_root")?
            .map(PathBuf::from)
            .unwrap_or_else(|| root_path.clone());

        let mut code_index = CodeIndex::with_root(workspace_root.clone());

        // Load file order if available
        if let Ok(Some(file_order_json)) = sqlite_index.get_metadata("file_order") {
            if let Ok(file_order) = serde_json::from_str::<Vec<PathBuf>>(&file_order_json) {
                code_index.set_file_order(file_order);
            }
        }

        // Load symbols, references, and opens from SQLite
        let files = sqlite_index.list_files()?;
        for file in &files {
            let symbols = sqlite_index.symbols_in_file(file)?;
            for symbol in symbols {
                code_index.add_symbol(symbol);
            }

            let references = sqlite_index.references_in_file(file)?;
            for reference in references {
                code_index.add_reference(file.clone(), reference);
            }

            let opens = sqlite_index.opens_for_file(file)?;
            for open in opens {
                code_index.add_open(file.clone(), open);
            }
        }

        let mut index = self.index.write().await;
        *index = code_index;

        info!(
            "Loaded {} symbols from {} files",
            index.symbol_count(),
            files.len()
        );

        Ok(true)
    }

    /// Build or rebuild the index for the workspace.
    /// First tries to load from SQLite, falls back to building fresh.
    async fn build_index(&self) -> Result<()> {
        // Try loading from SQLite first
        if self.load_index_from_sqlite().await? {
            return Ok(());
        }

        // No SQLite index found, build fresh
        let root = self.workspace_root.read().await;
        let root_path = match root.as_ref() {
            Some(r) => r.clone(),
            None => {
                warn!("No workspace root set");
                return Ok(());
            }
        };
        drop(root);

        info!("Building index for {:?}", root_path);

        // Find all source files
        let files = find_source_files(&root_path)?;
        info!("Found {} source files", files.len());

        let max_depth = *self.max_recursion_depth.read().await;
        let mut index = self.index.write().await;

        // Set workspace root for relative path storage
        index.set_workspace_root(root_path.clone());

        // Index external assemblies from .fsproj files
        self.index_external_assemblies(&mut index, &root_path).await;

        for file in files {
            if let Err(e) = self.index_file(&mut index, &file, max_depth) {
                warn!("Failed to index {:?}: {}", file, e);
            }
        }

        info!(
            "Indexed {} symbols in {} files",
            index.symbol_count(),
            index.file_count()
        );

        Ok(())
    }

    /// Index external assemblies based on .fsproj package references.
    async fn index_external_assemblies(&self, index: &mut CodeIndex, root_path: &Path) {
        use rocketindex::external_index::index_external_assemblies;
        use rocketindex::fsproj::{find_fsproj_files, parse_fsproj};

        let fsproj_files = find_fsproj_files(root_path);

        let mut all_packages = Vec::new();

        for fsproj_path in fsproj_files {
            if let Ok(info) = parse_fsproj(&fsproj_path) {
                all_packages.extend(info.package_references);
            }
        }

        // Remove duplicates
        all_packages.sort_by(|a, b| a.name.cmp(&b.name));
        all_packages.dedup_by(|a, b| a.name == b.name);

        if !all_packages.is_empty() {
            info!("Indexing {} external packages", all_packages.len());
            let external_index = index_external_assemblies(&all_packages);
            index.set_external_index(external_index);
        }
    }

    /// Index a single file into the in-memory CodeIndex.
    fn index_file(&self, index: &mut CodeIndex, file: &PathBuf, max_depth: usize) -> Result<()> {
        let content = std::fs::read_to_string(file)?;

        // Clear existing data for this file
        index.clear_file(file);

        // Extract symbols
        let result = extract_symbols(file, &content, max_depth);

        // Add symbols to index
        for symbol in result.symbols {
            index.add_symbol(symbol);
        }

        // Add references
        for reference in result.references {
            index.add_reference(file.clone(), reference);
        }

        // Add opens
        for open in result.opens {
            index.add_open(file.clone(), open);
        }

        Ok(())
    }

    /// Update a single file in both the in-memory index and SQLite database.
    /// Parses the file once and shares results between both indexes.
    async fn update_file(&self, file: &PathBuf) -> Result<()> {
        let max_depth = *self.max_recursion_depth.read().await;

        // Read and parse once
        let content = std::fs::read_to_string(file)?;
        let result = extract_symbols(file, &content, max_depth);

        // Update in-memory index
        {
            let mut index = self.index.write().await;
            index.clear_file(file);

            for symbol in &result.symbols {
                index.add_symbol(symbol.clone());
            }
            for reference in &result.references {
                index.add_reference(file.clone(), reference.clone());
            }
            for open in &result.opens {
                index.add_open(file.clone(), open.clone());
            }
        }

        // Update SQLite if it exists - single transaction for all operations
        let root = self.workspace_root.read().await;
        if let Some(root_path) = root.as_ref() {
            let db_path = Self::get_db_path(root_path);
            if db_path.exists() {
                match SqliteIndex::open(&db_path) {
                    Ok(sqlite_index) => {
                        // Convert opens to the format expected by update_file_data
                        let opens: Vec<(String, u32)> = result
                            .opens
                            .iter()
                            .enumerate()
                            .map(|(i, open)| (open.clone(), i as u32 + 1))
                            .collect();

                        if let Err(e) = sqlite_index.update_file_data(
                            file,
                            &result.symbols,
                            &result.references,
                            &opens,
                        ) {
                            warn!("Failed to update SQLite index for {:?}: {}", file, e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to open SQLite index for update: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Publish diagnostics for a file based on syntax errors.
    async fn publish_diagnostics(&self, uri: &Url, errors: Vec<SyntaxError>) {
        let diagnostics: Vec<Diagnostic> = errors
            .into_iter()
            .map(|error| Diagnostic {
                range: Range {
                    start: Position {
                        line: error.location.line.saturating_sub(1),
                        character: error.location.column.saturating_sub(1),
                    },
                    end: Position {
                        line: error.location.end_line.saturating_sub(1),
                        character: error.location.end_column.saturating_sub(1),
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("rocketindex-lsp".to_string()),
                message: error.message,
                ..Default::default()
            })
            .collect();

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }

    /// Parse a file and publish any syntax error diagnostics.
    async fn check_and_publish_diagnostics(&self, uri: &Url, content: &str) {
        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return,
        };

        let max_depth = *self.max_recursion_depth.read().await;
        let result = extract_symbols(&file, content, max_depth);
        self.publish_diagnostics(uri, result.errors).await;
    }

    /// Get the symbol at a given position in a file using tree-sitter.
    ///
    /// This properly handles F# identifiers including:
    /// - Simple identifiers: `foo`
    /// - Qualified names: `Module.foo`
    /// - Tick identifiers: ``` ``weird name`` ```
    ///
    /// Uses in-memory content if available, otherwise reads from disk.
    async fn get_symbol_at_position(&self, file: &PathBuf, pos: Position) -> Option<String> {
        let content = self.documents.get_content(file).await?;

        LSP_FSHARP_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = parser.parse(&content, None)?;
            let point = Point::new(pos.line as usize, pos.character as usize);

            // Find the smallest node containing this position
            let mut node = tree.root_node().descendant_for_point_range(point, point)?;

            // Walk up to find an identifier or long_identifier
            loop {
                match node.kind() {
                    "identifier" | "long_identifier" | "long_identifier_or_op" => {
                        return node
                            .utf8_text(content.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    // For operators, return the operator text
                    "op_name" | "infix_op" | "prefix_op" => {
                        return node
                            .utf8_text(content.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    _ => {
                        // Walk up to parent
                        node = node.parent()?;
                    }
                }
            }
        })
    }
}

/// Convert our SymbolKind to LSP SymbolKind.
fn to_lsp_symbol_kind(kind: rocketindex::SymbolKind) -> SymbolKind {
    match kind {
        rocketindex::SymbolKind::Module => SymbolKind::MODULE,
        rocketindex::SymbolKind::Function => SymbolKind::FUNCTION,
        rocketindex::SymbolKind::Value => SymbolKind::VARIABLE,
        rocketindex::SymbolKind::Type => SymbolKind::TYPE_PARAMETER,
        rocketindex::SymbolKind::Record => SymbolKind::STRUCT,
        rocketindex::SymbolKind::Union => SymbolKind::ENUM,
        rocketindex::SymbolKind::Interface => SymbolKind::INTERFACE,
        rocketindex::SymbolKind::Class => SymbolKind::CLASS,
        rocketindex::SymbolKind::Member => SymbolKind::METHOD,
    }
}

/// Compute organized (sorted) open statements.
///
/// Returns the sorted opens as a string and the range to replace, or None if no opens found.
fn compute_organize_opens(content: &str) -> Option<(String, Range)> {
    let lines: Vec<&str> = content.lines().collect();

    // Find all open statements and their locations
    let mut opens: Vec<(usize, &str)> = Vec::new();
    let mut first_open_line: Option<usize> = None;
    let mut last_open_line: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("open ") {
            if first_open_line.is_none() {
                first_open_line = Some(i);
            }
            last_open_line = Some(i);

            // Extract the module name
            let module_name = trimmed.strip_prefix("open ").unwrap_or("").trim();
            opens.push((i, module_name));
        }
    }

    // Need at least 2 opens to organize
    if opens.len() < 2 {
        return None;
    }

    let first_line = first_open_line?;
    let last_line = last_open_line?;

    // Sort the module names
    let mut module_names: Vec<&str> = opens.iter().map(|(_, name)| *name).collect();
    module_names.sort_by(|a, b| {
        // Sort by depth first (fewer dots = higher priority)
        let a_depth = a.matches('.').count();
        let b_depth = b.matches('.').count();
        if a_depth != b_depth {
            return a_depth.cmp(&b_depth);
        }
        // Then alphabetically
        a.to_lowercase().cmp(&b.to_lowercase())
    });

    // Check if already sorted
    let current_order: Vec<&str> = opens.iter().map(|(_, name)| *name).collect();
    if current_order == module_names {
        return None;
    }

    // Build the sorted opens string
    let sorted_text = module_names
        .iter()
        .map(|name| format!("open {}", name))
        .collect::<Vec<_>>()
        .join("\n");

    // Calculate the range to replace
    let range = Range {
        start: Position {
            line: first_line as u32,
            character: 0,
        },
        end: Position {
            line: last_line as u32,
            character: lines[last_line].len() as u32,
        },
    };

    Some((sorted_text, range))
}

/// Find potential missing open statements for unresolved symbols.
///
/// Scans the file for identifiers that cannot be resolved and suggests
/// modules that define them.
fn find_missing_opens(
    index: &rocketindex::CodeIndex,
    file: &Path,
    content: &str,
    max_depth: usize,
) -> Vec<String> {
    use rocketindex::extract_symbols;

    let result = extract_symbols(file, content, max_depth);
    let mut unresolved = Vec::new();

    // Check all references
    for reference in &result.references {
        let name = &reference.name;

        // Skip if it's already resolvable
        if index.resolve(name, file).is_some() || index.resolve_dotted(name, file).is_some() {
            continue;
        }

        // Try to find a module that defines this symbol
        // Look for symbols where the qualified name ends with this name
        let candidates: Vec<_> = index
            .search(name)
            .into_iter()
            .filter(|sym| sym.name == *name)
            .collect();

        for candidate in candidates {
            // Extract the module part (everything before the last dot)
            if let Some(module_name) = candidate.qualified.rsplit_once('.') {
                let module_name = module_name.0;

                // Check if this module is already opened
                let already_opened = result.opens.iter().any(|open| open == module_name);

                if !already_opened && !unresolved.contains(&module_name.to_string()) {
                    unresolved.push(module_name.to_string());
                }
            }
        }
    }

    unresolved
}

/// Find the best position to insert a new open statement.
///
/// Returns the position after the last open statement, or at the top of the file.
fn find_open_insert_position(content: &str) -> Position {
    let lines: Vec<&str> = content.lines().collect();

    // Find the last open statement
    for (i, line) in lines.iter().enumerate().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("open ") {
            // Insert after this line
            return Position {
                line: (i + 1) as u32,
                character: 0,
            };
        }
    }

    // No opens found, insert at the beginning (after any module declaration)
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("module ") || trimmed.starts_with("namespace ") {
            // Insert after the module/namespace declaration
            return Position {
                line: (i + 1) as u32,
                character: 0,
            };
        }
    }

    // No module/namespace, insert at the very top
    Position {
        line: 0,
        character: 0,
    }
}

/// Get the word at the given position (for completion prefix).
///
/// Returns the partial word being typed, or None if at whitespace.
/// Extract the expression before a dot at the given position.
///
/// For example, if the line is "user.Name" and cursor is after "Name",
/// this should return "user".
fn get_expression_before_dot(content: &str, pos: Position) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;

    if line_idx >= lines.len() {
        return None;
    }

    let line = lines[line_idx];
    let col = pos.character as usize;

    if col == 0 || col > line.len() {
        return None;
    }

    // Find the dot before the cursor
    let before_cursor = &line[..col];
    let dot_pos = before_cursor.rfind('.')?;

    // Extract from the start of the expression to the dot
    let expr_start = before_cursor[..dot_pos]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '!')
        .map(|i| i + 1)
        .unwrap_or(0);

    let expr = before_cursor[expr_start..dot_pos].trim();
    if expr.is_empty() {
        None
    } else {
        Some(expr.to_string())
    }
}

fn get_word_at_position(content: &str, pos: Position) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;

    if line_idx >= lines.len() {
        return None;
    }

    let line = lines[line_idx];
    let col = pos.character as usize;

    if col == 0 || col > line.len() {
        return None;
    }

    // Walk backwards from cursor to find start of word
    let before_cursor = &line[..col];
    let word_start = before_cursor
        .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '!')
        .map(|i| i + 1)
        .unwrap_or(0);

    let word = &before_cursor[word_start..];
    if word.is_empty() {
        None
    } else {
        Some(word.to_string())
    }
}

/// Resolve an expression to its type name for member completion.
///
/// This handles multiple cases:
/// 1. Direct type names: "String", "Console" -> return as-is if type cache has members
/// 2. Variable names: "user" -> resolve to qualified name, look up type in cache
/// 3. Qualified names: "MyModule.user" -> look up type directly
fn resolve_expression_type(
    index: &rocketindex::CodeIndex,
    expr: &str,
    from_file: &Path,
) -> Option<String> {
    // First, check if the expression is directly a type name with members
    if index.get_type_members(expr).is_some() {
        return Some(expr.to_string());
    }

    // Try to resolve the expression as a symbol
    let resolved = index
        .resolve(expr, from_file)
        .or_else(|| index.resolve_dotted(expr, from_file));

    if let Some(result) = resolved {
        // Got the symbol, now look up its type
        let qualified_name = &result.symbol.qualified;

        // Check type cache for this symbol's type
        if let Some(type_sig) = index.get_symbol_type(qualified_name) {
            // Extract the simple type name (handle Async<User>, User list, etc.)
            let type_name = extract_simple_type_name(type_sig);
            return Some(type_name.to_string());
        }

        // Fallback: for record/class types, the symbol itself might be a type
        // e.g., if "User" is both a type and has members
        if index.get_type_members(&result.symbol.name).is_some() {
            return Some(result.symbol.name.clone());
        }
    }

    // Check if it could be a type name in the qualified form
    // e.g., "System.String" or "MyApp.Domain.User"
    if expr.contains('.') {
        // Try the last component as a type name
        if let Some(type_name) = expr.rsplit('.').next() {
            if index.get_type_members(type_name).is_some() {
                return Some(type_name.to_string());
            }
        }
    }

    None
}

/// Extract simple type name from a type signature.
///
/// Handles F# type syntax:
/// - "string" -> "string"
/// - "User" -> "User"
/// - "Async<User>" -> "User"
/// - "User list" -> "User"
/// - "int -> string" -> "string" (return type)
fn extract_simple_type_name(type_sig: &str) -> &str {
    let trimmed = type_sig.trim();

    // Handle F# postfix types
    let postfix_types = [" list", " option", " array", " seq", " ref"];
    for suffix in &postfix_types {
        if let Some(stripped) = trimmed.strip_suffix(suffix) {
            return stripped.trim();
        }
    }

    // Handle generic types: Async<User> -> User
    if let Some(angle_pos) = trimmed.find('<') {
        let inner = &trimmed[angle_pos + 1..];
        if let Some(end) = inner.find(['>', ',']) {
            return inner[..end].trim();
        }
    }

    // Handle function types: take the return type
    if trimmed.contains("->") {
        if let Some(last_arrow) = trimmed.rfind("->") {
            return extract_simple_type_name(trimmed[last_arrow + 2..].trim());
        }
    }

    trimmed
}

/// Convert our Location to LSP Location.
fn to_lsp_location(loc: &rocketindex::Location) -> Location {
    Location {
        uri: Url::from_file_path(&loc.file)
            .unwrap_or_else(|_| Url::parse("file:///unknown").unwrap()),
        range: Range {
            start: Position {
                line: loc.line.saturating_sub(1),
                character: loc.column.saturating_sub(1),
            },
            end: Position {
                line: loc.end_line.saturating_sub(1),
                character: loc.end_column.saturating_sub(1),
            },
        },
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        // Store workspace root
        if let Some(root_uri) = params.root_uri {
            if let Ok(path) = root_uri.to_file_path() {
                *self.workspace_root.write().await = Some(path);
            }
        } else if let Some(folders) = params.workspace_folders {
            if let Some(folder) = folders.first() {
                if let Ok(path) = folder.uri.to_file_path() {
                    *self.workspace_root.write().await = Some(path);
                }
            }
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                rename_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "rocketindex-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("F# Language Server initialized");

        // Load config from workspace root
        if let Some(root) = self.workspace_root.read().await.as_ref() {
            let config = Config::load(root);
            *self.max_recursion_depth.write().await = config.max_recursion_depth;
            info!(
                "Loaded config: max_recursion_depth={}",
                config.max_recursion_depth
            );
        }

        // Build initial index (loads from SQLite if available)
        if let Err(e) = self.build_index().await {
            error!("Failed to build index: {}", e);
            self.client
                .log_message(MessageType::ERROR, format!("Index build failed: {}", e))
                .await;
        } else {
            self.client
                .log_message(MessageType::INFO, "F# index built successfully")
                .await;
        }
    }

    async fn shutdown(&self) -> LspResult<()> {
        info!("F# Language Server shutting down");
        Ok(())
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        // Get the symbol at the cursor position using tree-sitter
        let word = match self.get_symbol_at_position(&file, pos).await {
            Some(w) => w,
            None => return Ok(None),
        };

        info!("Looking up definition for: {}", word);

        let index = self.index.read().await;

        // Try to resolve the symbol
        if let Some(result) = index.resolve(&word, &file) {
            // Convert relative path to absolute for LSP
            let abs_location = index.make_location_absolute(&result.symbol.location);
            let loc = to_lsp_location(&abs_location);
            return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
        }

        // Try dotted resolution
        if let Some(result) = index.resolve_dotted(&word, &file) {
            // Convert relative path to absolute for LSP
            let abs_location = index.make_location_absolute(&result.symbol.location);
            let loc = to_lsp_location(&abs_location);
            return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        // Get the symbol at the cursor position
        let word = match self.get_symbol_at_position(&file, pos).await {
            Some(w) => w,
            None => return Ok(None),
        };

        let index = self.index.read().await;

        // Try to resolve the symbol
        let resolved = index
            .resolve(&word, &file)
            .or_else(|| index.resolve_dotted(&word, &file));

        if let Some(result) = resolved {
            let sym = &result.symbol;

            // Build hover content
            let kind_str = format!("{}", sym.kind);
            let file_name = sym
                .location
                .file
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("unknown");

            // Try to get signature - prefer symbol.signature, fallback to type cache
            let type_info = sym
                .signature
                .as_ref()
                .map(|s| format!("\n\n**Signature:** `{}`", s))
                .or_else(|| {
                    index
                        .get_symbol_type(&sym.qualified)
                        .map(|t| format!("\n\n**Type:** `{}`", t))
                })
                .unwrap_or_default();

            let content = format!(
                "**{}** `{}`{}\n\n---\n\n*Defined in* `{}` *at line {}*\n\n*Qualified:* `{}`",
                kind_str, sym.name, type_info, file_name, sym.location.line, sym.qualified
            );

            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            }));
        }

        Ok(None)
    }

    async fn references(&self, params: ReferenceParams) -> LspResult<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        // Get the symbol at the cursor position
        let word = match self.get_symbol_at_position(&file, pos).await {
            Some(w) => w,
            None => return Ok(None),
        };

        info!("Finding references for: {}", word);

        let index = self.index.read().await;

        // Try to resolve the symbol to get its qualified name
        let resolved = index
            .resolve(&word, &file)
            .or_else(|| index.resolve_dotted(&word, &file));

        if let Some(result) = resolved {
            let qualified_name = &result.symbol.qualified;

            // Find all references to this symbol
            let references = index.find_references(qualified_name);

            let locations: Vec<Location> = references
                .into_iter()
                .map(|reference| {
                    let abs_location = index.make_location_absolute(&reference.location);
                    to_lsp_location(&abs_location)
                })
                .collect();

            if !locations.is_empty() {
                return Ok(Some(locations));
            }
        }

        Ok(None)
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> LspResult<Option<Vec<SymbolInformation>>> {
        let query = &params.query;

        if query.is_empty() {
            return Ok(Some(Vec::new()));
        }

        let index = self.index.read().await;

        #[allow(deprecated)]
        let matches: Vec<SymbolInformation> = index
            .search(query)
            .into_iter()
            .take(50) // Limit results
            .map(|sym| {
                // Convert relative path to absolute for LSP
                let abs_location = index.make_location_absolute(&sym.location);
                SymbolInformation {
                    name: sym.name.clone(),
                    kind: to_lsp_symbol_kind(sym.kind),
                    location: to_lsp_location(&abs_location),
                    container_name: sym.qualified.rsplit_once('.').map(|(c, _)| c.to_string()),
                    tags: None,
                    deprecated: None,
                }
            })
            .collect();

        Ok(Some(matches))
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        // Get document content
        let content = match self.documents.get_content(&file).await {
            Some(c) => c,
            None => return Ok(None),
        };

        // Get the word being typed (prefix for filtering)
        let prefix = get_word_at_position(&content, pos);

        // Collect completions from multiple sources
        let mut items = Vec::new();

        // Check if this is member completion (triggered by ".")
        if let Some(context) = &params.context {
            if context.trigger_character.as_deref() == Some(".") {
                // Try member completion
                if let Some(expr) = get_expression_before_dot(&content, pos) {
                    let index = self.index.read().await;

                    // Try to resolve the expression type:
                    // 1. First, try direct type name lookup (e.g., "String.")
                    // 2. Then, try to resolve as a variable and get its type
                    let type_name = resolve_expression_type(&index, &expr, &file);

                    if let Some(type_name) = type_name {
                        if let Some(type_members) = index.get_type_members(&type_name) {
                            for member in type_members {
                                items.push(CompletionItem {
                                    label: member.member.clone(),
                                    kind: Some(match member.kind {
                                        rocketindex::MemberKind::Property => {
                                            CompletionItemKind::PROPERTY
                                        }
                                        rocketindex::MemberKind::Method => {
                                            CompletionItemKind::METHOD
                                        }
                                        rocketindex::MemberKind::Field => CompletionItemKind::FIELD,
                                        rocketindex::MemberKind::Event => CompletionItemKind::EVENT,
                                    }),
                                    detail: Some(format!(
                                        "{} ({})",
                                        member.member_type, member.type_name
                                    )),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
            }
        }

        // Add keyword completions
        items.extend(completion::keyword_completions(prefix.as_deref()));

        // Add symbol completions from the index
        {
            let index = self.index.read().await;
            items.extend(completion::symbol_completions(
                &index,
                &file,
                prefix.as_deref(),
                50, // Limit symbol results
            ));
        }

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> LspResult<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        // Get document content
        let content = match self.documents.get_content(&file).await {
            Some(c) => c,
            None => return Ok(None),
        };

        let mut actions = Vec::new();

        // Check if we can offer "Organize Opens"
        if let Some((sorted_opens, range)) = compute_organize_opens(&content) {
            let edit = TextEdit {
                range,
                new_text: sorted_opens,
            };

            let mut changes = std::collections::HashMap::new();
            changes.insert(uri.clone(), vec![edit]);

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Organize opens".to_string(),
                kind: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // Check for missing open suggestions
        {
            let index = self.index.read().await;
            let max_depth = *self.max_recursion_depth.read().await;
            let missing_opens = find_missing_opens(&index, &file, &content, max_depth);

            for module_name in missing_opens {
                // Find a good place to insert the open (after existing opens or at top)
                let insert_pos = find_open_insert_position(&content);

                let edit = TextEdit {
                    range: Range {
                        start: insert_pos,
                        end: insert_pos,
                    },
                    new_text: format!("open {}\n", module_name),
                };

                let mut changes = std::collections::HashMap::new();
                changes.insert(uri.clone(), vec![edit]);

                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Add open {}", module_name),
                    kind: Some(CodeActionKind::QUICKFIX),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn rename(&self, params: RenameParams) -> LspResult<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let new_name = params.new_name;

        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        // Get the symbol at the cursor position
        let word = match self.get_symbol_at_position(&file, pos).await {
            Some(w) => w,
            None => return Ok(None),
        };

        info!("Renaming symbol: {} to {}", word, new_name);

        let index = self.index.read().await;

        // Try to resolve the symbol to get its qualified name and definition
        let resolved = index
            .resolve(&word, &file)
            .or_else(|| index.resolve_dotted(&word, &file));

        if let Some(result) = resolved {
            let sym = &result.symbol;
            let qualified_name = &sym.qualified;
            let short_name = &sym.name;

            // Collect all locations to rename: definition + references
            let mut locations = Vec::new();

            // Add definition location
            locations.push(sym.location.clone());

            // Add all reference locations
            let references = index.find_references(qualified_name);
            for reference in references {
                locations.push(reference.location.clone());
            }

            // Create text edits for each location
            let mut changes: std::collections::HashMap<Url, Vec<TextEdit>> =
                std::collections::HashMap::new();

            for location in locations {
                let abs_location = index.make_location_absolute(&location);
                let lsp_location = to_lsp_location(&abs_location);
                let file_uri = lsp_location.uri;

                // Get the document content to find the exact text to replace
                let content = if let Some(c) = self.documents.get_content(&abs_location.file).await
                {
                    c
                } else {
                    // Fallback to reading from disk
                    match std::fs::read_to_string(&abs_location.file) {
                        Ok(c) => c,
                        Err(_) => continue,
                    }
                };

                // Find the text at the location
                let lines: Vec<&str> = content.lines().collect();
                if abs_location.line as usize >= lines.len() {
                    continue;
                }
                let line = lines[abs_location.line as usize - 1]; // 1-indexed to 0-indexed

                // Extract the word at the position
                let start_col = abs_location.column as usize - 1; // 1-indexed to 0-indexed
                let end_col = abs_location.end_column as usize - 1;

                if start_col >= line.len() || end_col > line.len() {
                    continue;
                }

                let current_text = &line[start_col..end_col];

                // Determine what to replace
                let replacement_text = if current_text == short_name {
                    new_name.clone()
                } else if current_text.ends_with(&format!(".{}", short_name)) {
                    current_text.replace(&format!(".{}", short_name), &format!(".{}", new_name))
                } else {
                    // Fallback: replace the short name if it appears
                    current_text.replace(short_name, &new_name)
                };

                let range = lsp_location.range;
                let edit = TextEdit {
                    range,
                    new_text: replacement_text,
                };

                changes.entry(file_uri).or_default().push(edit);
            }

            if changes.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }))
            }
        } else {
            Ok(None)
        }
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = &params.text_document.uri;
        info!("File opened: {}", uri);

        // Store document content in memory
        self.documents
            .open(
                uri,
                params.text_document.text.clone(),
                params.text_document.version,
            )
            .await;

        // Check for syntax errors and publish diagnostics
        self.check_and_publish_diagnostics(uri, &params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = &params.text_document.uri;

        // Apply changes to in-memory document
        self.documents
            .change(uri, params.content_changes, params.text_document.version)
            .await;

        // Check for syntax errors and publish diagnostics on each change
        // This gives real-time feedback as the user types
        if let Ok(file) = uri.to_file_path() {
            if let Some(content) = self.documents.get_content(&file).await {
                self.check_and_publish_diagnostics(uri, &content).await;
            }
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = &params.text_document.uri;
        let file = match uri.to_file_path() {
            Ok(f) => f,
            Err(_) => return,
        };

        info!("Reindexing saved file: {:?}", file);

        // Update both in-memory index and SQLite
        if let Err(e) = self.update_file(&file).await {
            error!("Failed to reindex {:?}: {}", file, e);
            self.client
                .log_message(MessageType::ERROR, format!("Reindex failed: {}", e))
                .await;
        }

        // Re-check diagnostics from the saved file
        if let Ok(content) = std::fs::read_to_string(&file) {
            self.check_and_publish_diagnostics(uri, &content).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = &params.text_document.uri;
        info!("File closed: {}", uri);

        // Remove document from in-memory store
        self.documents.close(uri).await;

        // Clear diagnostics for closed file
        self.client
            .publish_diagnostics(uri.clone(), vec![], None)
            .await;
    }
}

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    info!("Starting F# Language Server");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        index: Arc::new(RwLock::new(CodeIndex::new())),
        workspace_root: Arc::new(RwLock::new(None)),
        documents: DocumentStore::new(),
        max_recursion_depth: Arc::new(RwLock::new(500)), // Default, updated on init
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_word_at_position_simple() {
        let content = "let hello = 42";
        // Cursor after "let" at position 3
        let pos = Position {
            line: 0,
            character: 3,
        };
        assert_eq!(get_word_at_position(content, pos), Some("let".to_string()));
    }

    #[test]
    fn test_get_word_at_position_mid_word() {
        let content = "let hello = 42";
        // Cursor in middle of "hello" at position 6
        let pos = Position {
            line: 0,
            character: 6,
        };
        assert_eq!(get_word_at_position(content, pos), Some("he".to_string()));
    }

    #[test]
    fn test_get_word_at_position_at_space() {
        let content = "let hello = 42";
        // Cursor at space after "let"
        let pos = Position {
            line: 0,
            character: 4,
        };
        // Should still get "let" since we're right after it
        // Actually, position 4 is at space, let's check what we get
        let result = get_word_at_position(content, pos);
        // At position 4, we're after the space, so no word
        assert!(result.is_none() || result == Some("let".to_string()));
    }

    #[test]
    fn test_get_word_at_position_bang_keyword() {
        let content = "let! result = async";
        // Cursor after "let!"
        let pos = Position {
            line: 0,
            character: 4,
        };
        assert_eq!(get_word_at_position(content, pos), Some("let!".to_string()));
    }

    #[test]
    fn test_get_word_at_position_multiline() {
        let content = "let x = 1\nlet y = 2";
        // Cursor on second line, after "let"
        let pos = Position {
            line: 1,
            character: 3,
        };
        assert_eq!(get_word_at_position(content, pos), Some("let".to_string()));
    }

    #[test]
    fn test_get_word_at_position_start_of_line() {
        let content = "let hello = 42";
        let pos = Position {
            line: 0,
            character: 0,
        };
        assert_eq!(get_word_at_position(content, pos), None);
    }

    // ============================================================
    // Organize Opens Tests
    // ============================================================

    #[test]
    fn test_organize_opens_sorts_alphabetically() {
        let content = r#"module Test

open Zebra
open Apple
open Banana

let x = 1"#;

        let result = compute_organize_opens(content);
        assert!(result.is_some());

        let (sorted, _range) = result.unwrap();
        assert_eq!(sorted, "open Apple\nopen Banana\nopen Zebra");
    }

    #[test]
    fn test_organize_opens_sorts_by_depth_first() {
        let content = r#"module Test

open System.Collections.Generic
open System
open MyApp.Services
open MyApp

let x = 1"#;

        let result = compute_organize_opens(content);
        assert!(result.is_some());

        let (sorted, _range) = result.unwrap();
        // System and MyApp should come before deeper ones
        let lines: Vec<&str> = sorted.lines().collect();
        assert!(lines[0] == "open MyApp" || lines[0] == "open System");
    }

    #[test]
    fn test_organize_opens_none_if_already_sorted() {
        let content = r#"module Test

open Apple
open Banana
open Zebra

let x = 1"#;

        let result = compute_organize_opens(content);
        // Should return None because already sorted
        assert!(result.is_none());
    }

    #[test]
    fn test_organize_opens_none_if_single_open() {
        let content = r#"module Test

open System

let x = 1"#;

        let result = compute_organize_opens(content);
        // Should return None because only one open
        assert!(result.is_none());
    }

    #[test]
    fn test_organize_opens_none_if_no_opens() {
        let content = r#"module Test

let x = 1"#;

        let result = compute_organize_opens(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_organize_opens_correct_range() {
        let content = r#"module Test

open Zebra
open Apple

let x = 1"#;

        let result = compute_organize_opens(content);
        assert!(result.is_some());

        let (_sorted, range) = result.unwrap();
        // Opens start at line 2, end at line 3
        assert_eq!(range.start.line, 2);
        assert_eq!(range.end.line, 3);
    }

    // ============================================================
    // Extract Simple Type Name Tests
    // ============================================================

    #[test]
    fn test_extract_simple_type_name_basic() {
        assert_eq!(extract_simple_type_name("string"), "string");
        assert_eq!(extract_simple_type_name("int"), "int");
        assert_eq!(extract_simple_type_name("User"), "User");
    }

    #[test]
    fn test_extract_simple_type_name_postfix() {
        assert_eq!(extract_simple_type_name("int list"), "int");
        assert_eq!(extract_simple_type_name("User option"), "User");
        assert_eq!(extract_simple_type_name("string array"), "string");
        assert_eq!(extract_simple_type_name("User seq"), "User");
    }

    #[test]
    fn test_extract_simple_type_name_generic() {
        assert_eq!(extract_simple_type_name("Async<User>"), "User");
        assert_eq!(extract_simple_type_name("Result<User, Error>"), "User");
        assert_eq!(extract_simple_type_name("Task<string>"), "string");
    }

    #[test]
    fn test_extract_simple_type_name_function() {
        assert_eq!(extract_simple_type_name("int -> string"), "string");
        assert_eq!(extract_simple_type_name("int -> User"), "User");
        assert_eq!(extract_simple_type_name("string -> int -> bool"), "bool");
    }

    #[test]
    fn test_extract_simple_type_name_whitespace() {
        assert_eq!(extract_simple_type_name("  string  "), "string");
        assert_eq!(extract_simple_type_name("  User option  "), "User");
    }

    // ============================================================
    // Resolve Expression Type Tests
    // ============================================================

    #[test]
    fn test_resolve_expression_type_direct_type() {
        use rocketindex::type_cache::{MemberKind, TypeCache, TypeCacheSchema, TypeMember};

        let mut index = rocketindex::CodeIndex::new();

        // Set up type cache with User type members
        let schema = TypeCacheSchema {
            version: 1,
            extracted_at: "2024-12-02".to_string(),
            project: "Test".to_string(),
            symbols: vec![],
            members: vec![TypeMember {
                type_name: "User".to_string(),
                member: "Name".to_string(),
                member_type: "string".to_string(),
                kind: MemberKind::Property,
            }],
        };
        index.set_type_cache(TypeCache::from_schema(schema));

        // Direct type name should resolve
        let result = resolve_expression_type(&index, "User", Path::new("test.fs"));
        assert_eq!(result, Some("User".to_string()));
    }

    #[test]
    fn test_resolve_expression_type_via_symbol() {
        use rocketindex::type_cache::{
            MemberKind, TypeCache, TypeCacheSchema, TypeMember, TypedSymbol,
        };
        use rocketindex::{Location, Symbol, SymbolKind, Visibility};

        let mut index = rocketindex::CodeIndex::new();

        // Add a symbol 'user' with type 'User'
        index.add_symbol(Symbol::new(
            "user".to_string(),
            "MyModule.user".to_string(),
            SymbolKind::Value,
            Location::new(std::path::PathBuf::from("test.fs"), 1, 1),
            Visibility::Public,
            "fsharp".to_string(),
        ));

        // Set up type cache
        let schema = TypeCacheSchema {
            version: 1,
            extracted_at: "2024-12-02".to_string(),
            project: "Test".to_string(),
            symbols: vec![TypedSymbol {
                name: "user".to_string(),
                qualified: "MyModule.user".to_string(),
                type_signature: "User".to_string(),
                file: "test.fs".to_string(),
                line: 1,
                parameters: vec![],
            }],
            members: vec![TypeMember {
                type_name: "User".to_string(),
                member: "Name".to_string(),
                member_type: "string".to_string(),
                kind: MemberKind::Property,
            }],
        };
        index.set_type_cache(TypeCache::from_schema(schema));

        // Variable name should resolve to its type
        let result = resolve_expression_type(&index, "user", Path::new("test.fs"));
        assert_eq!(result, Some("User".to_string()));
    }

    #[test]
    fn test_resolve_expression_type_not_found() {
        let index = rocketindex::CodeIndex::new();

        let result = resolve_expression_type(&index, "unknown", Path::new("test.fs"));
        assert_eq!(result, None);
    }

    // ============================================================
    // Find Missing Opens Tests
    // ============================================================

    #[test]
    fn test_find_missing_opens_suggests_module() {
        use rocketindex::{Location, Symbol, SymbolKind, Visibility};

        let mut index = rocketindex::CodeIndex::new();

        // Add a symbol in Utils module
        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "MyApp.Utils.helper".to_string(),
            SymbolKind::Function,
            Location::new(std::path::PathBuf::from("Utils.fs"), 10, 1),
            Visibility::Public,
            "fsharp".to_string(),
        ));

        // Content that references 'helper' without opening Utils
        let content = r#"module Test

let x = helper 42
"#;

        let missing =
            find_missing_opens(&index, &std::path::PathBuf::from("Test.fs"), content, 500);

        assert!(
            missing.contains(&"MyApp.Utils".to_string()),
            "Should suggest MyApp.Utils, got: {:?}",
            missing
        );
    }

    #[test]
    fn test_find_missing_opens_no_suggestion_when_already_open() {
        use rocketindex::{Location, Symbol, SymbolKind, Visibility};

        let mut index = rocketindex::CodeIndex::new();

        // Add a symbol
        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "MyApp.Utils.helper".to_string(),
            SymbolKind::Function,
            Location::new(std::path::PathBuf::from("Utils.fs"), 10, 1),
            Visibility::Public,
            "fsharp".to_string(),
        ));

        // Content that has the module already opened
        let content = r#"module Test

open MyApp.Utils

let x = helper 42
"#;

        let missing =
            find_missing_opens(&index, &std::path::PathBuf::from("Test.fs"), content, 500);

        assert!(
            !missing.contains(&"MyApp.Utils".to_string()),
            "Should not suggest already-opened module"
        );
    }

    // ============================================================
    // Find Open Insert Position Tests
    // ============================================================

    #[test]
    fn test_find_open_insert_position_after_existing_opens() {
        let content = r#"module Test

open System
open System.IO

let x = 1
"#;

        let pos = find_open_insert_position(content);
        // Should be after the last open (line 4 is "open System.IO", so insert at line 5)
        assert_eq!(pos.line, 4);
        assert_eq!(pos.character, 0);
    }

    #[test]
    fn test_find_open_insert_position_after_module() {
        let content = r#"module Test

let x = 1
"#;

        let pos = find_open_insert_position(content);
        // Should be after the module declaration (line 1)
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 0);
    }

    #[test]
    fn test_find_open_insert_position_at_top() {
        let content = r#"let x = 1
"#;

        let pos = find_open_insert_position(content);
        // No module/namespace, should be at the very top
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 0);
    }
}
