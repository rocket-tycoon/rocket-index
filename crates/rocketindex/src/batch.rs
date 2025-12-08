//! Batch processing for file system events.
//!
//! This module provides efficient batch processing of file changes,
//! collecting events over a configurable window and processing them
//! in a single database transaction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::db::SqliteIndex;
use crate::watch::WatchEvent;
use crate::{extract_symbols, IndexError};

/// Default batch interval (how long to wait before flushing)
pub const DEFAULT_BATCH_INTERVAL: Duration = Duration::from_millis(100);

/// A batch processor that collects file events and processes them efficiently.
///
/// Instead of processing each file change individually, the batch processor:
/// 1. Collects events for a configurable time window
/// 2. Deduplicates paths (multiple changes to same file become one)
/// 3. Processes all changes in a single database transaction
///
/// # Example
/// ```ignore
/// let mut batch = BatchProcessor::new(Duration::from_millis(100));
///
/// // Add events as they arrive
/// batch.add_event(WatchEvent::Modified(path1.clone()));
/// batch.add_event(WatchEvent::Modified(path1.clone())); // Deduped
/// batch.add_event(WatchEvent::Modified(path2.clone()));
///
/// // Check if it's time to flush
/// if batch.should_flush() {
///     let stats = batch.flush(&index, max_depth)?;
///     println!("Processed {} files", stats.files_updated);
/// }
/// ```
pub struct BatchProcessor {
    /// Files that need to be re-indexed
    pending_updates: HashSet<PathBuf>,
    /// Files that need to be removed from the index
    pending_deletes: HashSet<PathBuf>,
    /// When the current batch started (first event after last flush)
    batch_start: Option<Instant>,
    /// How long to wait before flushing
    batch_interval: Duration,
    /// Maximum recursion depth for symbol extraction
    max_depth: usize,
}

/// Statistics from a batch flush operation
#[derive(Debug, Clone, Default)]
pub struct BatchStats {
    /// Number of files updated/re-indexed
    pub files_updated: usize,
    /// Number of files removed from index
    pub files_deleted: usize,
    /// Number of symbols inserted
    pub symbols_inserted: usize,
    /// Number of references inserted
    pub references_inserted: usize,
    /// Time taken to process the batch
    pub duration: Duration,
}

impl BatchProcessor {
    /// Create a new batch processor with the specified interval.
    pub fn new(batch_interval: Duration, max_depth: usize) -> Self {
        Self {
            pending_updates: HashSet::new(),
            pending_deletes: HashSet::new(),
            batch_start: None,
            batch_interval,
            max_depth,
        }
    }

    /// Create a new batch processor with default settings.
    pub fn with_defaults(max_depth: usize) -> Self {
        Self::new(DEFAULT_BATCH_INTERVAL, max_depth)
    }

    /// Add a watch event to the batch.
    ///
    /// Events are deduplicated: multiple modifications to the same file
    /// result in a single re-index operation.
    pub fn add_event(&mut self, event: WatchEvent) {
        // Start the batch timer on first event
        if self.batch_start.is_none() {
            self.batch_start = Some(Instant::now());
        }

        match event {
            WatchEvent::Created(path) | WatchEvent::Modified(path) => {
                // If file was marked for deletion, remove that
                self.pending_deletes.remove(&path);
                // Mark for update
                self.pending_updates.insert(path);
            }
            WatchEvent::Deleted(path) => {
                // If file was marked for update, remove that
                self.pending_updates.remove(&path);
                // Mark for deletion
                self.pending_deletes.insert(path);
            }
            WatchEvent::Renamed(old, new) => {
                // Old file is effectively deleted
                self.pending_updates.remove(&old);
                self.pending_deletes.insert(old);
                // New file needs to be indexed (if it exists)
                self.pending_deletes.remove(&new);
                self.pending_updates.insert(new);
            }
        }
    }

    /// Add multiple events to the batch.
    pub fn add_events(&mut self, events: impl IntoIterator<Item = WatchEvent>) {
        for event in events {
            self.add_event(event);
        }
    }

    /// Check if the batch should be flushed.
    ///
    /// Returns true if:
    /// - There are pending changes AND
    /// - The batch interval has elapsed since the first event
    pub fn should_flush(&self) -> bool {
        if self.is_empty() {
            return false;
        }

        if let Some(start) = self.batch_start {
            start.elapsed() >= self.batch_interval
        } else {
            false
        }
    }

    /// Check if there are any pending changes.
    pub fn is_empty(&self) -> bool {
        self.pending_updates.is_empty() && self.pending_deletes.is_empty()
    }

    /// Get the number of pending updates.
    pub fn pending_update_count(&self) -> usize {
        self.pending_updates.len()
    }

    /// Get the number of pending deletes.
    pub fn pending_delete_count(&self) -> usize {
        self.pending_deletes.len()
    }

    /// Flush the batch, processing all pending changes in a single transaction.
    ///
    /// Returns statistics about the flush operation.
    pub fn flush(&mut self, index: &SqliteIndex) -> Result<BatchStats, IndexError> {
        let flush_start = Instant::now();
        let mut stats = BatchStats::default();

        if self.is_empty() {
            return Ok(stats);
        }

        // Take ownership of pending sets
        let updates = std::mem::take(&mut self.pending_updates);
        let deletes = std::mem::take(&mut self.pending_deletes);

        // Reset batch timer
        self.batch_start = None;

        // Process deletes first (in case a file was renamed)
        for path in &deletes {
            if let Err(e) = index.clear_file(path) {
                tracing::warn!("Failed to clear file {:?}: {}", path, e);
            } else {
                stats.files_deleted += 1;
            }
        }

        // Process updates
        for path in &updates {
            // Skip if file doesn't exist (might have been deleted after the event)
            if !path.exists() {
                continue;
            }

            // Clear existing data for this file
            if let Err(e) = index.clear_file(path) {
                tracing::warn!("Failed to clear file {:?}: {}", path, e);
                continue;
            }

            // Read and parse the file
            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to read file {:?}: {}", path, e);
                    continue;
                }
            };

            let result = extract_symbols(path, &source, self.max_depth);

            // Insert symbols
            for symbol in &result.symbols {
                if let Err(e) = index.insert_symbol(symbol) {
                    tracing::warn!("Failed to insert symbol {}: {}", symbol.name, e);
                } else {
                    stats.symbols_inserted += 1;
                }
            }

            // Insert references
            for reference in &result.references {
                if let Err(e) = index.insert_reference(path, reference) {
                    tracing::warn!("Failed to insert reference: {}", e);
                } else {
                    stats.references_inserted += 1;
                }
            }

            // Insert opens
            for (line, open) in result.opens.iter().enumerate() {
                if let Err(e) = index.insert_open(path, open, line as u32 + 1) {
                    tracing::warn!("Failed to insert open: {}", e);
                }
            }

            stats.files_updated += 1;
        }

        stats.duration = flush_start.elapsed();
        Ok(stats)
    }

    /// Force an immediate flush regardless of the batch interval.
    pub fn force_flush(&mut self, index: &SqliteIndex) -> Result<BatchStats, IndexError> {
        self.flush(index)
    }

    /// Clear all pending changes without processing them.
    pub fn clear(&mut self) {
        self.pending_updates.clear();
        self.pending_deletes.clear();
        self.batch_start = None;
    }

    /// Get the paths that are pending update (for testing/debugging).
    pub fn pending_updates(&self) -> impl Iterator<Item = &Path> {
        self.pending_updates.iter().map(|p| p.as_path())
    }

    /// Get the paths that are pending deletion (for testing/debugging).
    pub fn pending_deletes(&self) -> impl Iterator<Item = &Path> {
        self.pending_deletes.iter().map(|p| p.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_processor_creation() {
        let batch = BatchProcessor::new(Duration::from_millis(100), 500);
        assert!(batch.is_empty());
        assert_eq!(batch.pending_update_count(), 0);
        assert_eq!(batch.pending_delete_count(), 0);
    }

    #[test]
    fn test_batch_processor_with_defaults() {
        let batch = BatchProcessor::with_defaults(500);
        assert!(batch.is_empty());
    }

    #[test]
    fn test_add_modified_event() {
        let mut batch = BatchProcessor::with_defaults(500);
        let path = PathBuf::from("/test/file.rs");

        batch.add_event(WatchEvent::Modified(path.clone()));

        assert!(!batch.is_empty());
        assert_eq!(batch.pending_update_count(), 1);
        assert_eq!(batch.pending_delete_count(), 0);
        assert!(batch.pending_updates().any(|p| p == path));
    }

    #[test]
    fn test_add_created_event() {
        let mut batch = BatchProcessor::with_defaults(500);
        let path = PathBuf::from("/test/new_file.rs");

        batch.add_event(WatchEvent::Created(path.clone()));

        assert_eq!(batch.pending_update_count(), 1);
        assert!(batch.pending_updates().any(|p| p == path));
    }

    #[test]
    fn test_add_deleted_event() {
        let mut batch = BatchProcessor::with_defaults(500);
        let path = PathBuf::from("/test/file.rs");

        batch.add_event(WatchEvent::Deleted(path.clone()));

        assert_eq!(batch.pending_delete_count(), 1);
        assert!(batch.pending_deletes().any(|p| p == path));
    }

    #[test]
    fn test_deduplication_multiple_modifies() {
        let mut batch = BatchProcessor::with_defaults(500);
        let path = PathBuf::from("/test/file.rs");

        // Same file modified multiple times
        batch.add_event(WatchEvent::Modified(path.clone()));
        batch.add_event(WatchEvent::Modified(path.clone()));
        batch.add_event(WatchEvent::Modified(path.clone()));

        // Should still be just one pending update
        assert_eq!(batch.pending_update_count(), 1);
    }

    #[test]
    fn test_modify_then_delete() {
        let mut batch = BatchProcessor::with_defaults(500);
        let path = PathBuf::from("/test/file.rs");

        batch.add_event(WatchEvent::Modified(path.clone()));
        batch.add_event(WatchEvent::Deleted(path.clone()));

        // Should be in deletes, not updates
        assert_eq!(batch.pending_update_count(), 0);
        assert_eq!(batch.pending_delete_count(), 1);
    }

    #[test]
    fn test_delete_then_create() {
        let mut batch = BatchProcessor::with_defaults(500);
        let path = PathBuf::from("/test/file.rs");

        batch.add_event(WatchEvent::Deleted(path.clone()));
        batch.add_event(WatchEvent::Created(path.clone()));

        // Should be in updates, not deletes (file was recreated)
        assert_eq!(batch.pending_update_count(), 1);
        assert_eq!(batch.pending_delete_count(), 0);
    }

    #[test]
    fn test_renamed_event() {
        let mut batch = BatchProcessor::with_defaults(500);
        let old_path = PathBuf::from("/test/old.rs");
        let new_path = PathBuf::from("/test/new.rs");

        batch.add_event(WatchEvent::Renamed(old_path.clone(), new_path.clone()));

        // Old should be deleted, new should be updated
        assert!(batch.pending_deletes().any(|p| p == old_path));
        assert!(batch.pending_updates().any(|p| p == new_path));
    }

    #[test]
    fn test_should_flush_empty() {
        let batch = BatchProcessor::with_defaults(500);
        assert!(!batch.should_flush());
    }

    #[test]
    fn test_should_flush_before_interval() {
        let mut batch = BatchProcessor::new(Duration::from_secs(10), 500);
        batch.add_event(WatchEvent::Modified(PathBuf::from("/test/file.rs")));

        // Immediately after adding, should not flush yet
        assert!(!batch.should_flush());
    }

    #[test]
    fn test_should_flush_after_interval() {
        let mut batch = BatchProcessor::new(Duration::from_millis(10), 500);
        batch.add_event(WatchEvent::Modified(PathBuf::from("/test/file.rs")));

        // Wait for interval to pass
        std::thread::sleep(Duration::from_millis(20));

        assert!(batch.should_flush());
    }

    #[test]
    fn test_clear() {
        let mut batch = BatchProcessor::with_defaults(500);
        batch.add_event(WatchEvent::Modified(PathBuf::from("/test/a.rs")));
        batch.add_event(WatchEvent::Deleted(PathBuf::from("/test/b.rs")));

        batch.clear();

        assert!(batch.is_empty());
        assert_eq!(batch.pending_update_count(), 0);
        assert_eq!(batch.pending_delete_count(), 0);
    }

    #[test]
    fn test_add_events_batch() {
        let mut batch = BatchProcessor::with_defaults(500);
        let events = vec![
            WatchEvent::Modified(PathBuf::from("/test/a.rs")),
            WatchEvent::Modified(PathBuf::from("/test/b.rs")),
            WatchEvent::Deleted(PathBuf::from("/test/c.rs")),
        ];

        batch.add_events(events);

        assert_eq!(batch.pending_update_count(), 2);
        assert_eq!(batch.pending_delete_count(), 1);
    }

    #[test]
    fn test_flush_resets_state() {
        let mut batch = BatchProcessor::with_defaults(500);
        batch.add_event(WatchEvent::Modified(PathBuf::from("/nonexistent/file.rs")));

        // Create an in-memory index for testing
        let index = SqliteIndex::in_memory().unwrap();

        let _ = batch.flush(&index);

        // After flush, batch should be empty
        assert!(batch.is_empty());
        assert!(!batch.should_flush());
    }

    #[test]
    fn test_batch_stats_default() {
        let stats = BatchStats::default();
        assert_eq!(stats.files_updated, 0);
        assert_eq!(stats.files_deleted, 0);
        assert_eq!(stats.symbols_inserted, 0);
        assert_eq!(stats.references_inserted, 0);
    }

    #[test]
    fn test_flush_empty_batch() {
        let mut batch = BatchProcessor::with_defaults(500);
        let index = SqliteIndex::in_memory().unwrap();

        let stats = batch.flush(&index).unwrap();

        assert_eq!(stats.files_updated, 0);
        assert_eq!(stats.files_deleted, 0);
    }

    #[test]
    fn test_flush_with_real_file() {
        use std::fs::File;
        use std::io::Write;

        let dir = tempfile::TempDir::new().unwrap();
        let test_file = dir.path().join("test.rs");

        // Create a simple Rust file
        {
            let mut file = File::create(&test_file).unwrap();
            writeln!(file, "fn hello() {{}}").unwrap();
        }

        let mut batch = BatchProcessor::with_defaults(500);
        batch.add_event(WatchEvent::Created(test_file));

        let index = SqliteIndex::in_memory().unwrap();
        let stats = batch.flush(&index).unwrap();

        assert_eq!(stats.files_updated, 1);
        assert!(stats.symbols_inserted > 0);
    }

    #[test]
    fn test_complex_event_sequence() {
        let mut batch = BatchProcessor::with_defaults(500);

        // Simulate: create file, modify it several times, then rename it
        let original = PathBuf::from("/test/original.rs");
        let renamed = PathBuf::from("/test/renamed.rs");

        batch.add_event(WatchEvent::Created(original.clone()));
        batch.add_event(WatchEvent::Modified(original.clone()));
        batch.add_event(WatchEvent::Modified(original.clone()));
        batch.add_event(WatchEvent::Renamed(original.clone(), renamed.clone()));

        // Original should be deleted, renamed should be updated
        assert_eq!(batch.pending_update_count(), 1);
        assert_eq!(batch.pending_delete_count(), 1);
        assert!(batch.pending_updates().any(|p| p == renamed));
        assert!(batch.pending_deletes().any(|p| p == original));
    }
}
