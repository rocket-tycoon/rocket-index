//! Multi-project state management for the MCP server.

use anyhow::{Context, Result};
use rocketindex::{CodeIndex, SqliteIndex};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::config::McpConfig;

/// State for a single indexed project
pub struct ProjectState {
    /// Root directory of the project
    pub root: PathBuf,
    /// SQLite index for persistent storage
    pub sqlite: SqliteIndex,
    /// In-memory CodeIndex for resolution
    pub code_index: CodeIndex,
    /// Whether the project has active watchers
    pub watching: bool,
}

// Safety: ProjectState is protected by a Mutex at the manager level
unsafe impl Send for ProjectState {}

impl ProjectState {
    /// Load a project from its root directory
    pub fn load(root: PathBuf) -> Result<Self> {
        let db_path = root.join(".rocketindex").join("index.db");
        if !db_path.exists() {
            anyhow::bail!(
                "No index found at {}. Run 'rkt index' in that directory first.",
                db_path.display()
            );
        }

        let sqlite = SqliteIndex::open(&db_path)
            .with_context(|| format!("Failed to open index at {}", db_path.display()))?;

        // Load CodeIndex from SQLite
        let mut code_index = CodeIndex::new();
        code_index.set_workspace_root(root.clone());

        // Load symbols into CodeIndex for resolution
        Self::load_code_index(&sqlite, &mut code_index)?;

        Ok(Self {
            root,
            sqlite,
            code_index,
            watching: false,
        })
    }

    /// Load symbols from SQLite into the in-memory CodeIndex
    fn load_code_index(sqlite: &SqliteIndex, code_index: &mut CodeIndex) -> Result<()> {
        // Get all files in the index
        let files = sqlite.list_files()?;

        for file in files {
            // Load symbols for this file
            let symbols = sqlite.symbols_in_file(&file)?;
            for symbol in symbols {
                code_index.add_symbol(symbol);
            }

            // Load references for this file
            let refs = sqlite.references_in_file(&file)?;
            for reference in refs {
                code_index.add_reference(file.clone(), reference);
            }

            // Load open statements for this file
            let opens = sqlite.opens_for_file(&file)?;
            for module in opens {
                code_index.add_open(file.clone(), module);
            }
        }

        Ok(())
    }

    /// Reload the index from SQLite
    pub fn reload(&mut self) -> Result<()> {
        let db_path = self.root.join(".rocketindex").join("index.db");
        self.sqlite = SqliteIndex::open(&db_path)?;
        self.code_index = CodeIndex::new();
        self.code_index.set_workspace_root(self.root.clone());
        Self::load_code_index(&self.sqlite, &mut self.code_index)?;
        Ok(())
    }
}

/// Multi-project state manager
pub struct ProjectManager {
    /// Active projects keyed by canonical root path
    /// Each ProjectState is wrapped in Mutex for thread-safe access
    projects: Arc<RwLock<HashMap<PathBuf, Mutex<ProjectState>>>>,
    /// Configuration
    config: Arc<RwLock<McpConfig>>,
}

impl ProjectManager {
    /// Create a new ProjectManager
    pub async fn new() -> Result<Self> {
        let config = McpConfig::load();
        let manager = Self {
            projects: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
        };

        // Load registered projects from config
        manager.load_registered_projects().await?;

        Ok(manager)
    }

    /// Load all registered projects from config
    async fn load_registered_projects(&self) -> Result<()> {
        let config = self.config.read().await;
        let project_paths: Vec<PathBuf> = config.projects.clone();
        drop(config);

        for path in project_paths {
            if let Err(e) = self.register(path.clone()).await {
                warn!("Failed to load project {}: {}", path.display(), e);
            }
        }

        Ok(())
    }

    /// Register a new project (or return existing)
    pub async fn register(&self, root: PathBuf) -> Result<()> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize path: {}", root.display()))?;

        // Check if already registered
        {
            let projects = self.projects.read().await;
            if projects.contains_key(&canonical) {
                info!("Project already registered: {}", canonical.display());
                return Ok(());
            }
        }

        // Load the project
        let state = ProjectState::load(canonical.clone())?;
        info!(
            "Registered project: {} ({} symbols)",
            canonical.display(),
            state.code_index.symbol_count()
        );

        // Add to active projects
        {
            let mut projects = self.projects.write().await;
            projects.insert(canonical.clone(), Mutex::new(state));
        }

        // Update config
        {
            let mut config = self.config.write().await;
            config.add_project(canonical);
            config.save()?;
        }

        Ok(())
    }

    /// Unregister a project
    #[allow(dead_code)]
    pub async fn unregister(&self, root: &Path) -> Result<bool> {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        let removed = {
            let mut projects = self.projects.write().await;
            projects.remove(&canonical).is_some()
        };

        if removed {
            let mut config = self.config.write().await;
            config.remove_project(&canonical);
            config.save()?;
            info!("Unregistered project: {}", canonical.display());
        }

        Ok(removed)
    }

    /// List all registered projects
    #[allow(dead_code)]
    pub async fn list_projects(&self) -> Vec<PathBuf> {
        let projects = self.projects.read().await;
        projects.keys().cloned().collect()
    }

    /// Find which project owns a file path
    pub async fn project_for_file(&self, file: &Path) -> Option<PathBuf> {
        let canonical = file.canonicalize().ok()?;
        let projects = self.projects.read().await;

        for root in projects.keys() {
            if canonical.starts_with(root) {
                return Some(root.clone());
            }
        }

        None
    }

    /// Get access to a project's state (locks the project's Mutex)
    pub async fn with_project<F, R>(&self, root: &Path, f: F) -> Option<R>
    where
        F: FnOnce(&ProjectState) -> R,
    {
        let canonical = root.canonicalize().ok()?;
        let projects = self.projects.read().await;
        projects.get(&canonical).map(|mutex| {
            let state = mutex.lock().expect("ProjectState mutex poisoned");
            f(&state)
        })
    }

    /// Get mutable access to a project's state (locks the project's Mutex)
    #[allow(dead_code)]
    pub async fn with_project_mut<F, R>(&self, root: &Path, f: F) -> Option<R>
    where
        F: FnOnce(&mut ProjectState) -> R,
    {
        let canonical = root.canonicalize().ok()?;
        let projects = self.projects.read().await;
        projects.get(&canonical).map(|mutex| {
            let mut state = mutex.lock().expect("ProjectState mutex poisoned");
            f(&mut state)
        })
    }

    /// Get all projects for symbol search (when no specific project is known)
    pub async fn all_projects(&self) -> Vec<PathBuf> {
        let projects = self.projects.read().await;
        projects.keys().cloned().collect()
    }

    /// Search for a symbol across all projects
    #[allow(dead_code)]
    pub async fn search_all_projects(
        &self,
        pattern: &str,
        limit: usize,
    ) -> Vec<(PathBuf, Vec<rocketindex::Symbol>)> {
        let projects = self.projects.read().await;
        let mut results = Vec::new();

        for (root, mutex) in projects.iter() {
            let state = mutex.lock().expect("ProjectState mutex poisoned");
            if let Ok(symbols) = state.sqlite.search(pattern, limit, None) {
                if !symbols.is_empty() {
                    results.push((root.clone(), symbols));
                }
            }
        }

        results
    }

    /// Find definition across all projects
    pub async fn find_definition_all(&self, symbol: &str) -> Vec<(PathBuf, rocketindex::Symbol)> {
        let projects = self.projects.read().await;
        let mut results = Vec::new();

        for (root, mutex) in projects.iter() {
            let state = mutex.lock().expect("ProjectState mutex poisoned");
            // Try exact match first
            if let Ok(Some(sym)) = state.sqlite.find_by_qualified(symbol) {
                results.push((root.clone(), sym));
            } else if let Ok(symbols) = state.sqlite.search(symbol, 10, None) {
                // Fall back to search
                for sym in symbols {
                    if sym.name == symbol || sym.qualified == symbol {
                        results.push((root.clone(), sym));
                    }
                }
            }
        }

        results
    }

    /// Check if a project is registered
    pub async fn has_project(&self, root: &Path) -> bool {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let projects = self.projects.read().await;
        projects.contains_key(&canonical)
    }

    /// Register a project with optional file watching
    pub async fn register_project(&self, root: PathBuf, _watch: bool) -> Result<()> {
        // For now, just delegate to register
        // TODO: Start file watcher if watch=true
        self.register(root).await
    }

    /// Reindex a project and return the symbol count
    pub async fn reindex_project(&self, root: &Path) -> Result<usize> {
        let canonical = root.canonicalize()?;

        // Run rkt index in the project directory
        let output = std::process::Command::new("rkt")
            .arg("index")
            .current_dir(&canonical)
            .output()
            .with_context(|| "Failed to run 'rkt index'")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("rkt index failed: {}", stderr);
        }

        // Reload the project state
        {
            let projects = self.projects.read().await;
            if let Some(mutex) = projects.get(&canonical) {
                let mut state = mutex.lock().expect("ProjectState mutex poisoned");
                state.reload()?;
                return Ok(state.code_index.symbol_count());
            }
        }

        anyhow::bail!("Project not found after reindex")
    }
}
