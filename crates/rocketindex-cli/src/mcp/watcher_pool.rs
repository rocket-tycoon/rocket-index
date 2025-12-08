//! File watcher pool for live reindexing.
//!
//! Each registered project gets its own file watcher that monitors for changes
//! and updates the index incrementally. Uses the same `DebouncedFileWatcher`
//! and `BatchProcessor` as the CLI `rkt watch` command.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use rocketindex::batch::{BatchProcessor, DEFAULT_BATCH_INTERVAL};
use rocketindex::config::Config;
use rocketindex::db::SqliteIndex;
use rocketindex::watch::DebouncedFileWatcher;

use super::ProjectManager;

/// Pool of file watchers, one per project.
///
/// Each project gets a dedicated watcher task that:
/// 1. Monitors for file changes using `DebouncedFileWatcher`
/// 2. Batches events using `BatchProcessor`
/// 3. Updates the SQLite index incrementally
/// 4. Signals the ProjectManager to reload the in-memory CodeIndex
pub struct WatcherPool {
    /// Reference to the project manager for reloading
    manager: Arc<ProjectManager>,
    /// Active watcher tasks keyed by project root
    watchers: Arc<RwLock<HashMap<PathBuf, WatcherHandle>>>,
    /// Debounce duration from config
    debounce_ms: u64,
}

/// Handle to a running watcher task
struct WatcherHandle {
    /// The tokio task running the watcher loop
    task: JoinHandle<()>,
    /// Signal to stop the watcher
    stop_signal: Arc<tokio::sync::Notify>,
}

impl WatcherPool {
    /// Create a new WatcherPool
    pub fn new(manager: Arc<ProjectManager>, debounce_ms: u64) -> Self {
        Self {
            manager,
            watchers: Arc::new(RwLock::new(HashMap::new())),
            debounce_ms,
        }
    }

    /// Start watching a project for file changes.
    ///
    /// Creates a dedicated task that monitors the project root and updates
    /// the index when files change.
    pub async fn start_watching(&self, root: PathBuf) -> anyhow::Result<()> {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());

        // Check if already watching
        {
            let watchers = self.watchers.read().await;
            if watchers.contains_key(&canonical) {
                debug!("Already watching: {}", canonical.display());
                return Ok(());
            }
        }

        // Get the index path
        let db_path = canonical.join(".rocketindex").join("index.db");
        if !db_path.exists() {
            anyhow::bail!(
                "No index found at {}. Run 'rkt index' first.",
                db_path.display()
            );
        }

        // Load config for max recursion depth
        let config = Config::load(&canonical);
        let max_depth = config.max_recursion_depth;

        // Create stop signal
        let stop_signal = Arc::new(tokio::sync::Notify::new());
        let stop_signal_clone = stop_signal.clone();

        // Clone what we need for the task
        let root_clone = canonical.clone();
        let debounce_duration = Duration::from_millis(self.debounce_ms);
        let manager = self.manager.clone();

        // Spawn the watcher task
        let task = tokio::spawn(async move {
            if let Err(e) = run_watcher_loop(
                root_clone.clone(),
                db_path,
                debounce_duration,
                max_depth,
                stop_signal_clone,
                manager,
            )
            .await
            {
                warn!(
                    "Watcher for {} exited with error: {}",
                    root_clone.display(),
                    e
                );
            }
        });

        // Store the handle
        {
            let mut watchers = self.watchers.write().await;
            watchers.insert(canonical.clone(), WatcherHandle { task, stop_signal });
        }

        info!("Started watching: {}", canonical.display());
        Ok(())
    }

    /// Stop watching a project
    #[allow(dead_code)]
    pub async fn stop_watching(&self, root: &std::path::Path) {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        let handle = {
            let mut watchers = self.watchers.write().await;
            watchers.remove(&canonical)
        };

        if let Some(handle) = handle {
            // Signal the task to stop
            handle.stop_signal.notify_one();
            // Wait for it to finish (with timeout)
            let _ = tokio::time::timeout(Duration::from_secs(2), handle.task).await;
            info!("Stopped watching: {}", canonical.display());
        }
    }

    /// Stop all watchers
    pub async fn stop_all(&self) {
        let handles: Vec<(PathBuf, WatcherHandle)> = {
            let mut watchers = self.watchers.write().await;
            watchers.drain().collect()
        };

        for (root, handle) in handles {
            handle.stop_signal.notify_one();
            let _ = tokio::time::timeout(Duration::from_secs(2), handle.task).await;
            info!("Stopped watching: {}", root.display());
        }
    }

    /// Check if a project is being watched
    #[allow(dead_code)]
    pub async fn is_watching(&self, root: &std::path::Path) -> bool {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let watchers = self.watchers.read().await;
        watchers.contains_key(&canonical)
    }

    /// Get the number of active watchers
    #[allow(dead_code)]
    pub async fn watcher_count(&self) -> usize {
        let watchers = self.watchers.read().await;
        watchers.len()
    }
}

/// Run the file watcher loop for a single project.
///
/// This runs in a dedicated tokio task and uses `spawn_blocking` for the
/// blocking file watcher operations.
async fn run_watcher_loop(
    root: PathBuf,
    db_path: PathBuf,
    debounce_duration: Duration,
    max_depth: usize,
    stop_signal: Arc<tokio::sync::Notify>,
    manager: Arc<ProjectManager>,
) -> anyhow::Result<()> {
    // Open the index - this is done in the async context since it's quick
    let index = SqliteIndex::open(&db_path)?;

    // Use a channel to communicate between blocking and async
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

    // Spawn the blocking watcher poll loop
    // All watcher operations happen in this single blocking task
    let root_clone = root.clone();
    let poll_handle = tokio::task::spawn_blocking(move || {
        // Create watcher and batch processor
        let mut watcher = match DebouncedFileWatcher::new(&root_clone, debounce_duration) {
            Ok(w) => w,
            Err(e) => {
                warn!(
                    "Failed to create watcher for {}: {}",
                    root_clone.display(),
                    e
                );
                return;
            }
        };

        if let Err(e) = watcher.start() {
            warn!(
                "Failed to start watcher for {}: {}",
                root_clone.display(),
                e
            );
            return;
        }

        let mut batch = BatchProcessor::new(DEFAULT_BATCH_INTERVAL, max_depth);

        loop {
            // Poll for events with timeout (allows checking stop signal)
            let events = watcher.wait_timeout(Duration::from_millis(100));

            if !events.is_empty() {
                batch.add_events(events);
            }

            // Check if we should flush
            if batch.should_flush() {
                match batch.flush(&index) {
                    Ok(stats) => {
                        if stats.files_updated > 0 || stats.files_deleted > 0 {
                            debug!(
                                "Updated index: {} files, {} symbols",
                                stats.files_updated, stats.symbols_inserted
                            );
                            // Signal that the index was updated
                            if event_tx
                                .blocking_send(IndexUpdateEvent {
                                    files_updated: stats.files_updated,
                                    files_deleted: stats.files_deleted,
                                })
                                .is_err()
                            {
                                // Channel closed, stop the loop
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to flush batch: {}", e);
                    }
                }
            }

            // Check stop signal via channel close
            if event_tx.is_closed() {
                break;
            }
        }

        // Stop the watcher
        let _ = watcher.stop();
    });

    // Handle events and stop signal in async context
    loop {
        tokio::select! {
            // Stop signal received
            _ = stop_signal.notified() => {
                debug!("Stop signal received for {}", root.display());
                // Close the channel to signal the blocking task to stop
                drop(event_rx);
                break;
            }
            // Index was updated
            Some(event) = event_rx.recv() => {
                if event.files_updated > 0 || event.files_deleted > 0 {
                    // Reload the in-memory CodeIndex
                    if manager.with_project_mut(&root, |state| {
                        if let Err(e) = state.reload() {
                            warn!("Failed to reload CodeIndex for {}: {}", root.display(), e);
                        }
                    }).await.is_some() {
                        debug!("Reloaded CodeIndex for {}", root.display());
                    }
                }
            }
        }
    }

    // Wait for the blocking task to finish
    let _ = tokio::time::timeout(Duration::from_secs(2), poll_handle).await;

    Ok(())
}

/// Event sent when the index is updated
struct IndexUpdateEvent {
    files_updated: usize,
    files_deleted: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_watcher_pool_creation() {
        let manager = Arc::new(ProjectManager::new().await.unwrap());
        let pool = WatcherPool::new(manager, 200);
        assert_eq!(pool.watcher_count().await, 0);
    }
}
