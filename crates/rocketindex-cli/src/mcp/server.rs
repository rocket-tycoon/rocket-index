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
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
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

/// Simple sliding window rate limiter for DoS protection.
///
/// SECURITY: Prevents resource exhaustion from rapid tool invocations.
/// Uses a sliding window of timestamps to track request rate.
struct RateLimiter {
    /// Timestamps of recent requests within the window
    timestamps: Mutex<VecDeque<Instant>>,
    /// Maximum requests allowed in the window
    max_requests: usize,
    /// Time window for rate limiting
    window: Duration,
}

impl RateLimiter {
    /// Create a new rate limiter
    fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            timestamps: Mutex::new(VecDeque::with_capacity(max_requests + 1)),
            max_requests,
            window,
        }
    }

    /// Check if a request is allowed. Returns true if allowed, false if rate limited.
    async fn check(&self) -> bool {
        let mut timestamps = self.timestamps.lock().await;
        let now = Instant::now();

        // Remove expired timestamps
        while let Some(&oldest) = timestamps.front() {
            if now.duration_since(oldest) > self.window {
                timestamps.pop_front();
            } else {
                break;
            }
        }

        // Check if we're at the limit
        if timestamps.len() >= self.max_requests {
            false
        } else {
            timestamps.push_back(now);
            true
        }
    }
}

/// RocketIndex MCP server
pub struct RocketIndexServer {
    manager: Arc<ProjectManager>,
    rate_limiter: RateLimiter,
}

impl RocketIndexServer {
    /// Maximum tool calls per second (rate limit)
    const RATE_LIMIT_REQUESTS: usize = 30;
    /// Rate limit window duration
    const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);

    /// Create a new RocketIndex MCP server
    pub fn new(manager: Arc<ProjectManager>) -> Self {
        Self {
            manager,
            rate_limiter: RateLimiter::new(Self::RATE_LIMIT_REQUESTS, Self::RATE_LIMIT_WINDOW),
        }
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
                "Finds all code locations that call a specific symbol. Use this tool when you need to understand the usage patterns of a function or method, or to assess the impact of changing a symbol. Unlike text search (grep), this tool understands code structure and returns precise caller locations, avoiding false positives from coincidentally named symbols.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "The fully qualified name of the symbol to find callers for (e.g., 'MyModule.MyFunction')."
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional path to the project root. If omitted, uses the current project context."
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            tool(
                "analyze_dependencies",
                "Analyzes the dependency graph starting from a specific symbol to understand code connectivity. Use this tool to trace what a function calls (forward analysis) or what calls a function (reverse analysis) up to a specified depth. This is essential for understanding complex control flows, architectural layering, and the ripple effects of changes. Grep cannot perform this graph traversal.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "The entry point symbol to connect dependencies from."
                        },
                        "depth": {
                            "type": "integer",
                            "description": "The maximum depth of the dependency graph to traverse. Default is 3."
                        },
                        "reverse": {
                            "type": "boolean",
                            "description": "If true, finds what calls this symbol (incoming edges). If false (default), finds what this symbol calls (outgoing edges)."
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional path to the project root."
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            // === MEDIUM VALUE: Better than grep for structured queries ===
            tool(
                "find_definition",
                "Locates the definition of a specific symbol. Use this tool to jump directly to the implementation-level code of a class, function, or variable. It is superior to grep for common names because it targets definitions specifically, filtering out usage noise. Provide a qualified name like 'Module.function' for the most precise results.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "The symbol name to define. Qualified names (e.g., 'MyModule.function') provide better accuracy."
                        },
                        "file": {
                            "type": "string",
                            "description": "The file path where the symbol is expected, if known. Helps disambiguate."
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional explicit project root."
                        },
                        "include_context": {
                            "type": "boolean",
                            "description": "If true, includes the source code lines around the definition. Default is false."
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            tool(
                "find_references",
                "Finds all code references to a symbol across the entire codebase. Use this tool when you need a comprehensive list of every place a symbol is used, not just callers. This tool differentiates between semantic references and mere text matches, making it more reliable than global text searches for refactoring or impact analysis.",
                json!({
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "The symbol to find usages of."
                        },
                        "project_root": {
                            "type": "string",
                            "description": "Optional project root."
                        },
                        "context_lines": {
                            "type": "integer",
                            "description": "Number of context lines to include around each reference. Default is 0."
                        }
                    },
                    "required": ["symbol"]
                }),
            ),
            tool(
                "describe_project",
                "Generates a semantic map of the project structure, listing files and top-level symbols (classes, modules, functions). Use this tool FIRST when entering a new codebase or directory to build a mental model of the architecture. It provides a structured hierarchy that is much easier to parse than raw file listings.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The path to the project root directory or subdirectory to describe."
                        }
                    },
                    "required": ["path"]
                }),
            ),
            tool(
                "search_symbols",
                "Searches for symbols matching a pattern. Use this tool when you don't know the exact name of a symbol, or to discover available API surfaces (e.g., all functions starting with 'User'). Supports wildcards (*) and fuzzy matching.",
                json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "The search pattern. Use '*' as a wildcard (e.g., '*Service')."
                        },
                        "language": {
                            "type": "string",
                            "description": "Optional filter by programming language (e.g., 'rust', 'typescript', 'fsharp')."
                        },
                        "fuzzy": {
                            "type": "boolean",
                            "description": "If true, performs a fuzzy match instead of a strict wildcard match. Default is false."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "The maximum number of results to return. Default is 20."
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
                "RocketIndex provides semantic code navigation tools that understand the structure of the code, not just text. \
                 ALWAYS prioritize these tools over generic text search (grep/ripgrep) for code inquiries.\n\n\
                 ## Tool Selection Strategy\n\
                 • **New to the code?** Start with `describe_project` to get a structural overview.\n\
                 • **Tracing logic?** Use `analyze_dependencies` to reverse-engineer how data flows through functions.\n\
                 • **Assessing impact?** Use `find_callers` or `find_references` to see what breaks if you change a symbol.\n\
                 • **Looking for something specific?** Use `find_definition` to jump to code, or `search_symbols` if you only know part of the name.\n\n\
                 Only fallback to grep if you are searching for literal strings (e.g. error messages, comments) that are not code symbols."
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
        let rate_limiter = &self.rate_limiter;
        async move {
            // SECURITY: Rate limiting to prevent DoS via tool spam
            if !rate_limiter.check().await {
                warn!("Rate limit exceeded for tool call");
                return Ok(CallToolResult::error(vec![Content::text(
                    "Rate limit exceeded. Please slow down requests.",
                )]));
            }

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
