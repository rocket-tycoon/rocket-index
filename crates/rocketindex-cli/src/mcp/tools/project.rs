//! Project management tools - register, list, reindex

use rmcp::model::{CallToolResult, Content};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::mcp::ProjectManager;

/// Input for register_project tool
#[derive(Debug, Deserialize)]
pub struct RegisterProjectInput {
    /// Path to the project root directory
    pub path: String,
    /// Enable file watching for this project
    #[serde(default = "default_watch")]
    pub watch: bool,
}

fn default_watch() -> bool {
    true
}

/// Input for reindex_project tool
#[derive(Debug, Deserialize)]
pub struct ReindexProjectInput {
    /// Path to the project root directory
    pub path: String,
}

/// Output for list_projects tool
#[derive(Debug, Serialize)]
pub struct ProjectInfo {
    pub path: String,
    pub symbol_count: usize,
    pub watching: bool,
    pub index_exists: bool,
}

/// Execute the register_project tool
pub async fn register_project(
    manager: Arc<ProjectManager>,
    input: RegisterProjectInput,
) -> CallToolResult {
    let path = PathBuf::from(&input.path);

    // Validate path exists
    if !path.exists() {
        return CallToolResult::error(vec![Content::text(format!(
            "Path '{}' does not exist.",
            input.path
        ))]);
    }

    if !path.is_dir() {
        return CallToolResult::error(vec![Content::text(format!(
            "Path '{}' is not a directory.",
            input.path
        ))]);
    }

    // Canonicalize the path
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return CallToolResult::error(vec![Content::text(format!(
                "Failed to resolve path '{}': {}",
                input.path, e
            ))]);
        }
    };

    // Check if already registered
    if manager.has_project(&canonical).await {
        return CallToolResult::success(vec![Content::text(format!(
            "Project '{}' is already registered.",
            canonical.display()
        ))]);
    }

    // Register the project
    match manager
        .register_project(canonical.clone(), input.watch)
        .await
    {
        Ok(()) => {
            let mut response =
                format!("Project '{}' registered successfully.", canonical.display());

            // Check if index exists
            let index_path = canonical.join(".rocketindex/index.db");
            if !index_path.exists() {
                response.push_str("\n\nNote: No index found. Run `reindex_project` to build the index, or use `rkt index` from the command line.");
            }

            if input.watch {
                response.push_str(
                    "\n\nFile watching enabled - index will update automatically as files change.",
                );
            }

            CallToolResult::success(vec![Content::text(response)])
        }
        Err(e) => CallToolResult::error(vec![Content::text(format!(
            "Failed to register project: {}",
            e
        ))]),
    }
}

/// Execute the list_projects tool
pub async fn list_projects(manager: Arc<ProjectManager>) -> CallToolResult {
    let projects = manager.all_projects().await;

    if projects.is_empty() {
        return CallToolResult::success(vec![Content::text(
            "No projects registered. Use `register_project` to add a project.",
        )]);
    }

    let mut infos = Vec::new();
    for root in projects {
        let info = manager
            .with_project(&root, |state| ProjectInfo {
                path: root.display().to_string(),
                symbol_count: state.sqlite.count_symbols().unwrap_or(0),
                watching: state.watching,
                index_exists: root.join(".rocketindex/index.db").exists(),
            })
            .await
            .unwrap_or(ProjectInfo {
                path: root.display().to_string(),
                symbol_count: 0,
                watching: false,
                index_exists: false,
            });

        infos.push(info);
    }

    let json = serde_json::to_string(&infos).unwrap_or_default();
    CallToolResult::success(vec![Content::text(json)])
}

/// Execute the reindex_project tool
pub async fn reindex_project(
    manager: Arc<ProjectManager>,
    input: ReindexProjectInput,
) -> CallToolResult {
    let path = PathBuf::from(&input.path);

    // Canonicalize the path
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return CallToolResult::error(vec![Content::text(format!(
                "Failed to resolve path '{}': {}",
                input.path, e
            ))]);
        }
    };

    // Check if registered
    if !manager.has_project(&canonical).await {
        return CallToolResult::error(vec![Content::text(format!(
            "Project '{}' is not registered. Use `register_project` first.",
            canonical.display()
        ))]);
    }

    // Perform reindex
    match manager.reindex_project(&canonical).await {
        Ok(symbol_count) => CallToolResult::success(vec![Content::text(format!(
            "Reindexed '{}' successfully. {} symbols indexed.",
            canonical.display(),
            symbol_count
        ))]),
        Err(e) => CallToolResult::error(vec![Content::text(format!(
            "Failed to reindex project: {}",
            e
        ))]),
    }
}
