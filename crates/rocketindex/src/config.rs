//! Configuration for RocketIndex.
//!
//! Loads settings from `.rocketindex.toml` in the project root.

use serde::Deserialize;
use std::path::Path;

/// Default directories to exclude from indexing.
pub const DEFAULT_EXCLUDE_DIRS: &[&str] = &[
    "node_modules",
    "bin",
    "obj",
    "packages",
    ".git",
    ".vs",
    ".idea",
];

/// RocketIndex configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Additional directories to exclude from indexing (merged with defaults).
    #[serde(default)]
    pub exclude_dirs: Vec<String>,

    /// Maximum recursion depth for parsing (default: 500).
    #[serde(default = "default_recursion_depth")]
    pub max_recursion_depth: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude_dirs: Vec::new(),
            max_recursion_depth: default_recursion_depth(),
        }
    }
}

fn default_recursion_depth() -> usize {
    500
}

impl Config {
    /// Load configuration from `.rocketindex.toml` in the given root directory.
    ///
    /// Returns default config if the file doesn't exist.
    pub fn load(root: &Path) -> Self {
        let config_path = root.join(".rocketindex.toml");
        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => {
                        tracing::info!("Loaded config from {:?}", config_path);
                        return config;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse {:?}: {}", config_path, e);
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to read {:?}: {}", config_path, e);
                }
            }
        }
        Self::default()
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
}
