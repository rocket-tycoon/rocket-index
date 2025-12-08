//! File watcher pool for live reindexing.
//!
//! TODO: Implement file watching for automatic index updates.
//! This is a stub implementation for now.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use super::ProjectManager;

/// Pool of file watchers, one per project
#[allow(dead_code)]
pub struct WatcherPool {
    /// Reference to the project manager
    _manager: Arc<ProjectManager>,
    /// Active watcher tasks keyed by project root
    watchers: Arc<RwLock<HashMap<PathBuf, JoinHandle<()>>>>,
}

#[allow(dead_code)]
impl WatcherPool {
    /// Create a new WatcherPool
    pub fn new(manager: Arc<ProjectManager>) -> Self {
        Self {
            _manager: manager,
            watchers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start watching a project for file changes
    pub async fn start_watching(&self, _root: PathBuf) -> anyhow::Result<()> {
        // TODO: Implement file watching
        // For now, index updates must be triggered manually via reindex_project
        Ok(())
    }

    /// Stop watching a project
    pub async fn stop_watching(&self, root: &PathBuf) {
        let mut watchers = self.watchers.write().await;
        if let Some(handle) = watchers.remove(root) {
            handle.abort();
        }
    }

    /// Stop all watchers
    pub async fn stop_all(&self) {
        let mut watchers = self.watchers.write().await;
        for (_, handle) in watchers.drain() {
            handle.abort();
        }
    }
}
