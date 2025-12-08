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
    fn tools() -> Vec<Tool> {
        vec![
            tool(
                "find_definition",
                "Find where a symbol is defined. Returns file path, line number, and optionally source context. Use qualified names like 'Module.function' for precise results.",
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
                "find_callers",
                "Find all locations that call a symbol. Returns the calling function and its location. Essential for understanding how a function is used.",
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
                "find_references",
                "Find all references to a symbol across the codebase. Returns every location where the symbol is used.",
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
                "search_symbols",
                "Search for symbols by pattern. Supports wildcards (*) and fuzzy matching. Use to discover available functions, classes, and types.",
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
            tool(
                "analyze_dependencies",
                "Analyze the dependency graph starting from a symbol. Shows what a function calls (forward) or what calls it (reverse). Useful for impact analysis.",
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
                "Get comprehensive information about a symbol including definition, callers, callees, git blame, and source context. Best for debugging and understanding complex code.",
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
            tool(
                "register_project",
                "Register a project directory with the MCP server. The project must have been previously indexed with 'rkt index'.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the project root directory"
                        },
                        "watch": {
                            "type": "boolean",
                            "description": "Enable file watching for this project (default: true)"
                        }
                    },
                    "required": ["path"]
                }),
            ),
            tool(
                "list_projects",
                "List all registered projects with their symbol counts and status.",
                json!({
                    "type": "object",
                    "properties": {}
                }),
            ),
            tool(
                "reindex_project",
                "Force a full reindex of a project. Use when files have changed significantly or the index seems stale.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the project root directory"
                        }
                    },
                    "required": ["path"]
                }),
            ),
            tool(
                "describe_project",
                "Get a comprehensive semantic map of the project. Lists files and top-level symbols (classes, modules) to help you understand the project structure.",
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
                "RocketIndex provides fast code navigation for multi-language codebases. \
                 Use find_definition to locate symbol definitions, find_callers to see what calls a function, \
                 and search_symbols to discover available symbols. Always prefer these tools over grep/search \
                 for code navigation tasks."
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

                "register_project" => {
                    let input: tools::RegisterProjectInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::register_project(manager, input).await)
                }

                "list_projects" => Ok(tools::list_projects(manager).await),

                "reindex_project" => {
                    let input: tools::ReindexProjectInput = serde_json::from_value(args)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
                    Ok(tools::reindex_project(manager, input).await)
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
