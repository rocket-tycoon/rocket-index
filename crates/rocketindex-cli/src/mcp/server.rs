//! MCP server implementation for RocketIndex.
//!
//! This module implements the Model Context Protocol (MCP) server that exposes
//! rocketindex code navigation tools to AI assistants.

use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use super::config::McpConfig;
use super::tools;
use super::watcher_pool::WatcherPool;
use super::ProjectManager;

/// Helper to convert JSON value to Arc<JsonObject> for tool schemas
fn json_schema(value: serde_json::Value) -> Arc<serde_json::Map<String, serde_json::Value>> {
    match value {
        serde_json::Value::Object(map) => Arc::new(map),
        _ => Arc::new(serde_json::Map::new()),
    }
}

/// Helper to create a tool definition
fn tool(name: &'static str, description: &'static str, schema: serde_json::Value) -> Tool {
    Tool {
        name: name.into(),
        title: None,
        description: Some(description.into()),
        input_schema: json_schema(schema),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
    }
}

/// RocketIndex MCP server
pub struct RocketIndexServer {
    manager: Arc<ProjectManager>,
}

impl RocketIndexServer {
    /// Create a new RocketIndex MCP server
    pub fn new(manager: Arc<ProjectManager>) -> Self {
        Self { manager }
    }

    /// Build the list of available tools
    /// Build the list of available tools.
    ///
    /// Tools are ordered by value-add: highest-value tools that grep cannot replicate
    /// are listed first, so AI assistants discover them before falling back to grep.
    fn tools() -> Vec<Tool> {
        vec![
            // === HIGH VALUE: Unique capabilities grep cannot provide ===
            tool(
                "find_callers",
                "Find all locations that call a symbol. USE THIS INSTEAD OF GREP for 'who calls X' queries - grep finds text matches but cannot distinguish callers from other references. Returns the calling function name and location.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Symbol to find callers for (qualified name preferred)"
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional project root"
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            tool(
                "analyze_dependencies",
                "Analyze the dependency graph starting from a symbol. THIS IS UNIQUE TO RKT - grep cannot traverse call graphs. Shows what a function calls (forward) or what calls it (reverse). Essential for impact analysis and understanding code flow.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Entry point symbol to start from"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Max depth to traverse (default: 3)"
                        },
                        "reverse": {
                            "type": "boolean",
                            "description": "Reverse direction (find what leads TO this symbol instead of FROM)"
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional project root"
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            tool(
                "enrich_symbol",
                "Get comprehensive information about a symbol: definition, callers, callees, git blame, and source context in one call. BEST FOR DEEP INVESTIGATION - aggregates data that would require multiple grep searches.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Symbol to enrich (qualified name preferred)"
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional project root"
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            // === MEDIUM VALUE: Better than grep for structured queries ===
            tool(
                "find_definition",
                "Find where a symbol is defined. PREFER OVER GREP for symbols with common names - returns the precise definition location, not just text matches. Use qualified names like 'Module.function' for best results.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Symbol name (qualified like 'MyModule.function' or short like 'function')"
                        },
                        "file": {
                            "type": "string",
                            "description": "Optional file context for resolution hints"
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional explicit project root"
                        },
                        "include_context": {
                            "type": "boolean",
                            "description": "Include source context line (default: false)"
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            tool(
                "find_references",
                "Find all references to a symbol across the codebase. Returns semantically-aware results - understands actual symbol references vs coincidental text matches.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Symbol to find usages of"
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional project root"
                        },
                        "context_lines": {
                            "type": "integer",
                            "description": "Context lines around each reference (default: 0)"
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            tool(
                "describe_project",
                "Get a semantic map of the project structure. Lists files and top-level symbols (classes, modules, functions). USE THIS FIRST when exploring an unfamiliar codebase - provides structured overview that grep cannot.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the project root directory (or subdirectory)"
                        }
                    },
                    "required": ["path"]
                }),
            ),
            tool(
                "search_symbols",
                "Search for symbols by pattern with wildcards (*) and fuzzy matching. Use to discover available functions, classes, and types when you don't know exact names.",
                json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Pattern to match (supports * wildcards)"
                        },
                        "language": {
                            "type": "string",
                            "description": "Filter by language (e.g., 'rust', 'typescript', 'fsharp')"
                        },
                        "fuzzy": {
                            "type": "boolean",
                            "description": "Use fuzzy matching (default: false)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum results per project (default: 20)"
                        }
                    },
                    "required": ["pattern"]
                }),
            ),
        ]
    }
}

impl ServerHandler for RocketIndexServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities {
                tools: Some(rmcp::model::ToolsCapability {
                    list_changed: Some(false),
                }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "rocketindex".into(),
                title: Some("RocketIndex Code Navigator".into()),
                version: env!("CARGO_PKG_VERSION").into(),
                icons: None,
                website_url: Some("https://github.com/rocket-tycoon/rocket-index".into()),
            },
            instructions: Some(
                "RocketIndex provides semantic code navigation. ALWAYS PREFER THESE TOOLS OVER GREP:\n\n\
                 ## Core Tools\n\
                 • find_callers: For 'who calls X?' - grep cannot distinguish callers from other text matches\n\
                 • analyze_dependencies: For call graph traversal - grep cannot do this at all\n\
                 • enrich_symbol: For comprehensive symbol info in one call\n\
                 • find_definition: For precise definition location, especially for common names\n\n\
                 ## Stacktrace Analysis Workflow\n\
                 When analyzing a stacktrace, use these tools together:\n\
                 1. Use enrich_symbol on the error location for source context and git blame\n\
                 2. Use find_callers on key frames to understand call patterns\n\
                 3. Use analyze_dependencies to trace the full call path\n\n\
                 Use grep only as a last resort for literal text search when these tools don't apply."
                    .into(),
            ),
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async move {
            Ok(ListToolsResult {
                tools: Self::tools(),
                next_cursor: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let manager = self.manager.clone();
        async move {
            let name = request.name.as_ref();
            let args = request
                .arguments
                .map(serde_json::Value::Object)
                .unwrap_or(serde_json::Value::Object(Default::default()));

            info!("Calling tool: {} with args: {}", name, args);

            match name {
                "find_definition" => {
                    let input: tools::FindDefinitionInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::find_definition(manager, input).await)
                }

                "find_callers" => {
                    let input: tools::FindCallersInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::find_callers(manager, input).await)
                }

                "find_references" => {
                    let input: tools::FindReferencesInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::find_references(manager, input).await)
                }

                "search_symbols" => {
                    let input: tools::SearchSymbolsInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::search_symbols(manager, input).await)
                }

                "analyze_dependencies" => {
                    let input: tools::AnalyzeDepsInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::analyze_dependencies(manager, input).await)
                }

                "enrich_symbol" => {
                    let input: tools::EnrichSymbolInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::enrich_symbol(manager, input).await)
                }

                "describe_project" => {
                    let input: tools::DescribeProjectInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::describe_project(manager, input).await)
                }

                _ => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Unknown tool: {}",
                    name
                ))])),
            }
        }
    }
}

/// Run the MCP server on stdio
pub async fn run_server(manager: Arc<ProjectManager>) -> anyhow::Result<()> {
    // Load config for watcher settings
    let config = McpConfig::load();

    // Create watcher pool if auto_watch is enabled
    let watcher_pool = if config.auto_watch {
        let pool = WatcherPool::new(manager.clone(), config.debounce_ms);

        // Start watching all registered projects
        for project in manager.all_projects().await {
            if let Err(e) = pool.start_watching(project.clone()).await {
                warn!("Failed to start watching {}: {}", project.display(), e);
            }
        }

        Some(pool)
    } else {
        None
    };

    let server = RocketIndexServer::new(manager);
    let transport = rmcp::transport::stdio();

    info!("Starting RocketIndex MCP server...");

    let running = server.serve(transport).await?;

    // Wait for the server to complete
    running.waiting().await?;

    // Stop all watchers on shutdown
    if let Some(pool) = watcher_pool {
        info!("Stopping file watchers...");
        pool.stop_all().await;
    }

    Ok(())
}
