//! MCP server configuration.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// MCP Server configuration stored at ~/.config/rocketindex/mcp.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Explicitly registered project roots
    #[serde(default)]
    pub projects: Vec<PathBuf>,

    /// Whether to automatically start file watchers for registered projects
    #[serde(default = "default_auto_watch")]
    pub auto_watch: bool,

    /// Debounce duration for file watching (milliseconds)
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            auto_watch: default_auto_watch(),
            debounce_ms: default_debounce_ms(),
        }
    }
}

fn default_auto_watch() -> bool {
    true
}

fn default_debounce_ms() -> u64 {
    200
}

impl McpConfig {
    /// Get the config file path
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rocketindex")
            .join("mcp.json")
    }

    /// Load config from disk, or return default if not found
    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    /// Save config to disk
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Add a project to the config
    pub fn add_project(&mut self, root: PathBuf) {
        let canonical = root.canonicalize().unwrap_or(root);
        if !self.projects.contains(&canonical) {
            self.projects.push(canonical);
        }
    }

    /// Remove a project from the config
    pub fn remove_project(&mut self, root: &Path) -> bool {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if let Some(pos) = self.projects.iter().position(|p| p == &canonical) {
            self.projects.remove(pos);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = McpConfig::default();
        assert!(config.projects.is_empty());
        assert!(config.auto_watch);
        assert_eq!(config.debounce_ms, 200);
    }

    #[test]
    fn test_add_remove_project() {
        let mut config = McpConfig::default();
        let path = PathBuf::from("/tmp/test-project");

        config.add_project(path.clone());
        assert_eq!(config.projects.len(), 1);

        // Adding same project again should not duplicate
        config.add_project(path.clone());
        assert_eq!(config.projects.len(), 1);

        assert!(config.remove_project(&path));
        assert!(config.projects.is_empty());
    }
}
