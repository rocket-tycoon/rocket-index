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
    ///
    /// SECURITY: Sets restrictive permissions (0600) to protect project paths from other users.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            // Set directory permissions to 0700 (owner only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, &content)?;

        // SECURITY: Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    /// Add a project to the config
    ///
    /// SECURITY: Only accepts canonicalizable paths to prevent symlink attacks.
    /// Returns true if added, false if already exists or path cannot be canonicalized.
    pub fn add_project(&mut self, root: PathBuf) -> bool {
        match root.canonicalize() {
            Ok(canonical) => {
                if !self.projects.contains(&canonical) {
                    self.projects.push(canonical);
                    true
                } else {
                    false // Already exists
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Cannot add project '{}': path canonicalization failed: {}",
                    root.display(),
                    e
                );
                false
            }
        }
    }

    /// Remove a project from the config
    ///
    /// SECURITY: Requires path to be canonicalizable.
    pub fn remove_project(&mut self, root: &Path) -> bool {
        match root.canonicalize() {
            Ok(canonical) => {
                if let Some(pos) = self.projects.iter().position(|p| p == &canonical) {
                    self.projects.remove(pos);
                    true
                } else {
                    false
                }
            }
            Err(_) => {
                // Try matching the raw path as fallback for removal only
                // This allows removing projects whose paths no longer exist
                if let Some(pos) = self.projects.iter().position(|p| p == root) {
                    self.projects.remove(pos);
                    true
                } else {
                    false
                }
            }
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
