//! Multi-project state management for the MCP server.

use anyhow::{Context, Result};
use rayon::prelude::*;
use rocketindex::config::Config;
use rocketindex::watch::find_source_files_with_config;
use rocketindex::{CodeIndex, SqliteIndex};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
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
    /// Whether the project has active watchers (managed by WatcherPool)
    #[allow(dead_code)]
    pub watching: bool,
}

impl ProjectState {
    /// Load a project from its root directory, auto-indexing if needed
    pub fn load(root: PathBuf) -> Result<Self> {
        let db_path = root.join(".rocketindex").join("index.db");

        // Auto-index if no index exists
        if !db_path.exists() {
            Self::create_index(&root)?;
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

    /// Create the index for a project (auto-indexing on first load)
    fn create_index(root: &Path) -> Result<()> {
        let start = Instant::now();
        info!("Auto-indexing project: {}", root.display());

        // Load configuration
        let config = Config::load(root);
        let exclude_dirs = config.excluded_dirs();

        // Find source files
        let files = find_source_files_with_config(root, &exclude_dirs, config.respect_gitignore)
            .with_context(|| format!("Failed to find source files in {}", root.display()))?;

        let max_depth = config.max_recursion_depth;

        // Parse files in parallel
        let parse_results: Vec<_> = files
            .par_iter()
            .filter_map(|file| match std::fs::read_to_string(file) {
                Ok(source) => {
                    let result = rocketindex::extract_symbols(file, &source, max_depth);
                    Some((file.clone(), result))
                }
                Err(e) => {
                    warn!("Failed to read {}: {}", file.display(), e);
                    None
                }
            })
            .collect();

        // Create index directory
        let index_dir = root.join(".rocketindex");
        std::fs::create_dir_all(&index_dir).with_context(|| {
            format!(
                "Failed to create .rocketindex directory at {}",
                index_dir.display()
            )
        })?;

        // SECURITY: Set directory permissions to 0700 (owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&index_dir, std::fs::Permissions::from_mode(0o700));
        }

        let db_path = index_dir.join("index.db");

        // Create SQLite index
        let index = SqliteIndex::create(&db_path)
            .with_context(|| format!("Failed to create index at {}", db_path.display()))?;

        // Store workspace root in metadata
        index
            .set_metadata("workspace_root", &root.to_string_lossy())
            .context("Failed to set workspace root")?;

        // Collect all data for batch insertion
        let mut all_symbols = Vec::new();
        let mut all_references: Vec<(PathBuf, rocketindex::index::Reference)> = Vec::new();
        let mut all_opens: Vec<(PathBuf, String, u32)> = Vec::new();

        for (file, parse_result) in parse_results {
            all_symbols.extend(parse_result.symbols);

            for reference in parse_result.references {
                all_references.push((file.clone(), reference));
            }

            for (line, open) in parse_result.opens.into_iter().enumerate() {
                all_opens.push((file.clone(), open, line as u32 + 1));
            }
        }

        let symbol_count = all_symbols.len();

        // Batch insert symbols
        index
            .insert_symbols(&all_symbols)
            .context("Failed to insert symbols")?;

        // Batch insert references
        let ref_tuples: Vec<_> = all_references
            .iter()
            .map(|(f, r)| (f.as_path(), r))
            .collect();
        index
            .insert_references(&ref_tuples)
            .context("Failed to insert references")?;

        // Batch insert opens
        let open_tuples: Vec<_> = all_opens
            .iter()
            .map(|(f, m, l)| (f.as_path(), m.as_str(), *l))
            .collect();
        index
            .insert_opens(&open_tuples)
            .context("Failed to insert opens")?;

        let duration = start.elapsed();
        info!(
            "Auto-indexed {} files, {} symbols in {:.2}s",
            files.len(),
            symbol_count,
            duration.as_secs_f64()
        );

        Ok(())
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
            // Skip non-existent paths (stale entries from previous runs)
            if !path.exists() {
                continue;
            }
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

    /// Register a project without persisting to config (for testing)
    #[cfg(test)]
    pub async fn register_in_memory(&self, root: PathBuf) -> Result<()> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize path: {}", root.display()))?;
        let state = ProjectState::load(canonical.clone())?;
        let mut projects = self.projects.write().await;
        projects.insert(canonical.clone(), Mutex::new(state));
        Ok(())
    }

    /// Create a ProjectManager without loading from global config (for testing)
    #[cfg(test)]
    pub async fn new_empty() -> Result<Self> {
        Ok(Self {
            projects: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(McpConfig::default())),
        })
    }

    /// Unregister a project
    #[allow(dead_code)]
    pub async fn unregister(&self, root: &Path) -> Result<bool> {
        // For unregister, we allow non-canonical paths to support removing stale entries
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

    /// Fuzzy search across all projects
    pub async fn fuzzy_search_all_projects(
        &self,
        pattern: &str,
        limit: usize,
    ) -> Vec<(PathBuf, Vec<rocketindex::Symbol>)> {
        let projects = self.projects.read().await;
        let mut results = Vec::new();

        for (root, mutex) in projects.iter() {
            let state = mutex.lock().expect("ProjectState mutex poisoned");
            // Distance 3 allows for typos
            if let Ok(fuzzy_results) = state.sqlite.fuzzy_search(pattern, 3, limit, None) {
                if !fuzzy_results.is_empty() {
                    let symbols = fuzzy_results.into_iter().map(|(s, _)| s).collect();
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

    /// Get the default project based on CWD.
    ///
    /// Returns the registered project containing the current working directory,
    /// or None if CWD is not inside any registered project.
    ///
    /// SECURITY: We do NOT JIT-register projects. Users must explicitly register
    /// projects with 'rkt serve add' or 'rkt index' to prevent arbitrary directory access.
    pub async fn default_project(&self) -> Option<PathBuf> {
        let cwd = std::env::current_dir().ok()?;

        // Return the registered project containing CWD, if any
        self.project_for_file(&cwd).await
    }

    /// Get project roots for a tool invocation.
    ///
    /// Priority:
    /// 1. Explicit project_root parameter (resolves to containing registered project if subdirectory)
    /// 2. Project containing the file parameter (if provided)
    /// 3. CWD project (via default_project)
    /// 4. All registered projects (fallback for multi-project scenarios)
    pub async fn resolve_projects(
        &self,
        explicit_root: Option<&str>,
        file_hint: Option<&str>,
    ) -> Vec<PathBuf> {
        // 1. Explicit project root - resolve to containing registered project if subdirectory
        if let Some(root) = explicit_root {
            let path = std::path::Path::new(root);
            // Check if this path is inside a registered project
            if let Some(project_root) = self.project_for_file(path).await {
                return vec![project_root];
            }
            // Security: Do NOT return unregistered paths - this prevents arbitrary file access
            // via malicious project_root parameters (e.g., "/etc", "~/.ssh")
            warn!(
                "Rejected access to unregistered project path: {}",
                path.display()
            );
            return vec![];
        }

        // 2. File hint
        if let Some(file) = file_hint {
            if let Some(root) = self.project_for_file(std::path::Path::new(file)).await {
                return vec![root];
            }
        }

        // 3. CWD project
        if let Some(root) = self.default_project().await {
            return vec![root];
        }

        // 4. All projects (fallback)
        self.all_projects().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_resolve_projects_subdirectory_maps_to_parent() {
        // Create a temp directory structure: /parent/child/grandchild
        let temp = TempDir::new().unwrap();
        let parent = temp.path().to_path_buf();
        let child = parent.join("child");
        let grandchild = child.join("grandchild");
        std::fs::create_dir_all(&grandchild).unwrap();

        // Create index in parent (makes it a valid project)
        let index_dir = parent.join(".rocketindex");
        std::fs::create_dir_all(&index_dir).unwrap();
        let db_path = index_dir.join("index.db");
        let _ = rocketindex::SqliteIndex::create(&db_path).unwrap();

        // Register the parent project (in-memory, doesn't persist to config)
        let manager = ProjectManager::new().await.unwrap();
        manager.register_in_memory(parent.clone()).await.unwrap();

        // Resolve with subdirectory path should return parent project
        let grandchild_str = grandchild.to_string_lossy().to_string();
        let resolved = manager.resolve_projects(Some(&grandchild_str), None).await;

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].canonicalize().unwrap(),
            parent.canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn test_resolve_projects_unregistered_path_rejected() {
        let manager = ProjectManager::new().await.unwrap();

        // Security: Unregistered paths should be rejected (return empty vec)
        // This prevents arbitrary file access via malicious project_root parameters
        let unregistered = "/some/unregistered/path";
        let resolved = manager.resolve_projects(Some(unregistered), None).await;

        assert!(
            resolved.is_empty(),
            "Unregistered paths must be rejected for security"
        );
    }
}
