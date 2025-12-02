//! fsharp-lsp: A minimal F# language server for Zed
//!
//! This server provides:
//! - Go-to-definition
//! - Workspace symbol search
//! - Incremental file indexing on save
//! - In-memory document tracking for unsaved changes
//!
//! Storage: Uses SQLite database (.fsharp-index/index.db) for persistence,
//! loaded into memory as CodeIndex for fast resolution.

mod document_store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use document_store::DocumentStore;
use fsharp_index::{
    db::DEFAULT_DB_NAME, extract_symbols, watch::find_fsharp_files, CodeIndex, SqliteIndex,
};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{error, info, warn};
use tree_sitter::{Parser, Point};

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
}

impl Backend {
    /// Get the path to the SQLite database.
    fn get_db_path(root: &Path) -> PathBuf {
        root.join(".fsharp-index").join(DEFAULT_DB_NAME)
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

        let files = find_fsharp_files(&root_path)?;
        info!("Found {} F# files", files.len());

        let mut index = self.index.write().await;

        // Set workspace root for relative path storage
        index.set_workspace_root(root_path.clone());

        for file in files {
            if let Err(e) = self.index_file(&mut index, &file) {
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

    /// Index a single file into the in-memory CodeIndex.
    fn index_file(&self, index: &mut CodeIndex, file: &PathBuf) -> Result<()> {
        let content = std::fs::read_to_string(file)?;

        // Clear existing data for this file
        index.clear_file(file);

        // Extract symbols
        let result = extract_symbols(file, &content);

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
    async fn update_file(&self, file: &PathBuf) -> Result<()> {
        // Update in-memory index
        {
            let mut index = self.index.write().await;
            self.index_file(&mut index, file)?;
        }

        // Also update SQLite if it exists
        let root = self.workspace_root.read().await;
        if let Some(root_path) = root.as_ref() {
            let db_path = Self::get_db_path(root_path);
            if db_path.exists() {
                match SqliteIndex::open(&db_path) {
                    Ok(sqlite_index) => {
                        // Clear existing data for this file
                        if let Err(e) = sqlite_index.clear_file(file) {
                            warn!("Failed to clear file from SQLite index: {}", e);
                        }

                        // Re-extract and insert
                        if let Ok(content) = std::fs::read_to_string(file) {
                            let result = extract_symbols(file, &content);

                            for symbol in &result.symbols {
                                if let Err(e) = sqlite_index.insert_symbol(symbol) {
                                    warn!("Failed to persist symbol {}: {}", symbol.name, e);
                                }
                            }
                            for reference in &result.references {
                                if let Err(e) = sqlite_index.insert_reference(file, reference) {
                                    warn!("Failed to persist reference: {}", e);
                                }
                            }
                            for (line, open) in result.opens.iter().enumerate() {
                                if let Err(e) = sqlite_index.insert_open(file, open, line as u32 + 1)
                                {
                                    warn!("Failed to persist open statement: {}", e);
                                }
                            }
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

        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .ok()?;

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
    }
}

/// Convert our SymbolKind to LSP SymbolKind.
fn to_lsp_symbol_kind(kind: fsharp_index::SymbolKind) -> SymbolKind {
    match kind {
        fsharp_index::SymbolKind::Module => SymbolKind::MODULE,
        fsharp_index::SymbolKind::Function => SymbolKind::FUNCTION,
        fsharp_index::SymbolKind::Value => SymbolKind::VARIABLE,
        fsharp_index::SymbolKind::Type => SymbolKind::TYPE_PARAMETER,
        fsharp_index::SymbolKind::Record => SymbolKind::STRUCT,
        fsharp_index::SymbolKind::Union => SymbolKind::ENUM,
        fsharp_index::SymbolKind::Interface => SymbolKind::INTERFACE,
        fsharp_index::SymbolKind::Class => SymbolKind::CLASS,
        fsharp_index::SymbolKind::Member => SymbolKind::METHOD,
    }
}

/// Convert our Location to LSP Location.
fn to_lsp_location(loc: &fsharp_index::Location) -> Location {
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
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "fsharp-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("F# Language Server initialized");

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

            // Try to get type signature from type cache if available
            let type_info = index
                .get_symbol_type(&sym.qualified)
                .map(|t| format!("\n\n**Type:** `{}`", t))
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

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = &params.text_document.uri;
        info!("File opened: {}", uri);

        // Store document content in memory
        self.documents
            .open(uri, params.text_document.text, params.text_document.version)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = &params.text_document.uri;

        // Apply changes to in-memory document
        self.documents
            .change(uri, params.content_changes, params.text_document.version)
            .await;

        // Note: We could trigger a debounced reindex here for live updates,
        // but for now we only reindex on save to match the original behavior.
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let file = match params.text_document.uri.to_file_path() {
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
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = &params.text_document.uri;
        info!("File closed: {}", uri);

        // Remove document from in-memory store
        self.documents.close(uri).await;
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
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
