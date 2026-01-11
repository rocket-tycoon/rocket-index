//! Configuration for RocketIndex.
//!
//! Loads settings from `.rocketindex.toml` in the project root.
//! Uses figment for layered configuration with provenance tracking.

use figment::{
    providers::{Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Default directories to exclude from indexing.
///
/// Note: `packages` was removed because pnpm/npm/yarn workspaces use it for
/// source code. NuGet packages contain mostly binaries that aren't indexed anyway.
pub const DEFAULT_EXCLUDE_DIRS: &[&str] = &[
    "node_modules",
    "bin",
    "obj",
    ".git",
    ".vs",
    ".idea",
    "target",
    "dist",
];

/// RocketIndex configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Additional directories to exclude from indexing (merged with defaults).
    #[serde(default)]
    pub exclude_dirs: Vec<String>,

    /// Maximum recursion depth for parsing (default: 500).
    #[serde(default = "default_recursion_depth")]
    pub max_recursion_depth: usize,

    /// Whether to respect .gitignore files when indexing (default: true).
    #[serde(default = "default_respect_gitignore")]
    pub respect_gitignore: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude_dirs: Vec::new(),
            max_recursion_depth: default_recursion_depth(),
            respect_gitignore: default_respect_gitignore(),
        }
    }
}

fn default_recursion_depth() -> usize {
    500
}

fn default_respect_gitignore() -> bool {
    true
}

impl Config {
    /// Load configuration from `.rocketindex.toml` in the given root directory.
    ///
    /// Uses figment for layered configuration with better error messages.
    /// Returns default config if the file doesn't exist.
    /// Reports parse errors with file, line, and key information.
    pub fn load(root: &Path) -> Self {
        let config_path = root.join(".rocketindex.toml");

        // Build layered config: defaults <- toml file
        let figment = Figment::from(Serialized::defaults(Config::default()));

        // Only add TOML provider if file exists
        let figment = if config_path.exists() {
            figment.merge(Toml::file(&config_path))
        } else {
            figment
        };

        match figment.extract() {
            Ok(config) => {
                if config_path.exists() {
                    tracing::info!("Loaded config from {:?}", config_path);
                }
                config
            }
            Err(e) => {
                // Figment provides detailed error messages with provenance
                tracing::warn!("Config error: {}", e);
                Self::default()
            }
        }
    }

    /// Get all directories to exclude (defaults + user-configured).
    pub fn excluded_dirs(&self) -> Vec<&str> {
        let mut dirs: Vec<&str> = DEFAULT_EXCLUDE_DIRS.to_vec();
        for dir in &self.exclude_dirs {
            if !dirs.contains(&dir.as_str()) {
                dirs.push(dir.as_str());
            }
        }
        dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.exclude_dirs.is_empty());
        assert_eq!(config.max_recursion_depth, 500);
        let excluded = config.excluded_dirs();
        assert!(excluded.contains(&"node_modules"));
        assert!(excluded.contains(&"bin"));
        assert!(excluded.contains(&"obj"));
    }

    #[test]
    fn test_load_missing_config() {
        let temp = TempDir::new().unwrap();
        let config = Config::load(temp.path());
        assert!(config.exclude_dirs.is_empty());
    }

    #[test]
    fn test_load_config() {
        let temp = TempDir::new().unwrap();
        let config_content = r#"
exclude_dirs = ["fcs-fable", "vendor"]
"#;
        std::fs::write(temp.path().join(".rocketindex.toml"), config_content).unwrap();

        let config = Config::load(temp.path());
        assert_eq!(config.exclude_dirs, vec!["fcs-fable", "vendor"]);

        let excluded = config.excluded_dirs();
        assert!(excluded.contains(&"fcs-fable"));
        assert!(excluded.contains(&"vendor"));
        assert!(excluded.contains(&"node_modules")); // default still present
    }

    #[test]
    fn test_load_config_with_max_recursion_depth() {
        let temp = TempDir::new().unwrap();
        let config_content = r#"
max_recursion_depth = 1000
"#;
        std::fs::write(temp.path().join(".rocketindex.toml"), config_content).unwrap();

        let config = Config::load(temp.path());
        assert_eq!(config.max_recursion_depth, 1000);
        assert!(config.exclude_dirs.is_empty()); // default for exclude_dirs
    }

    #[test]
    fn test_invalid_config_returns_defaults() {
        let temp = TempDir::new().unwrap();
        // Invalid: max_recursion_depth should be a number, not a string
        let config_content = r#"
max_recursion_depth = "not a number"
"#;
        std::fs::write(temp.path().join(".rocketindex.toml"), config_content).unwrap();

        // Should return defaults when config is invalid
        let config = Config::load(temp.path());
        assert_eq!(config.max_recursion_depth, 500); // default value
    }

    #[test]
    fn test_partial_config_merges_with_defaults() {
        let temp = TempDir::new().unwrap();
        // Only specify one field, others should come from defaults
        let config_content = r#"
respect_gitignore = false
"#;
        std::fs::write(temp.path().join(".rocketindex.toml"), config_content).unwrap();

        let config = Config::load(temp.path());
        assert!(!config.respect_gitignore); // from config
        assert_eq!(config.max_recursion_depth, 500); // from defaults
        assert!(config.exclude_dirs.is_empty()); // from defaults
    }
}
