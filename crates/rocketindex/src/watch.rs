//! File system watcher for incremental index updates.
//!
//! This module provides functionality to watch source files for changes
//! and trigger incremental re-indexing when files are modified.
//!
//! Two watcher implementations are provided:
//! - `FileWatcher`: Simple, low-level watcher (legacy)
//! - `DebouncedFileWatcher`: Recommended watcher with event debouncing and rename tracking

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::event::{DataChange, ModifyKind};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};

/// Events emitted by the file watcher.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A file was created
    Created(PathBuf),
    /// A file was modified
    Modified(PathBuf),
    /// A file was deleted
    Deleted(PathBuf),
    /// A file was renamed (old path, new path)
    Renamed(PathBuf, PathBuf),
}

/// File system watcher for F# source files.
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<Result<Event, notify::Error>>,
    root: PathBuf,
}

impl FileWatcher {
    /// Create a new file watcher for the given root directory.
    ///
    /// # Arguments
    /// * `root` - The root directory to watch for changes
    ///
    /// # Returns
    /// A new `FileWatcher` instance or an error if watching could not be started.
    pub fn new(root: &Path) -> Result<Self, notify::Error> {
        let (tx, rx) = channel();

        let config = Config::default().with_poll_interval(Duration::from_secs(1));

        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            config,
        )?;

        Ok(Self {
            watcher,
            receiver: rx,
            root: root.to_path_buf(),
        })
    }

    /// Start watching the root directory.
    pub fn start(&mut self) -> Result<(), notify::Error> {
        self.watcher.watch(&self.root, RecursiveMode::Recursive)
    }

    /// Stop watching the root directory.
    pub fn stop(&mut self) -> Result<(), notify::Error> {
        self.watcher.unwatch(&self.root)
    }

    /// Poll for the next file system event.
    ///
    /// This is a non-blocking call that returns `None` if no events are available.
    pub fn poll(&self) -> Option<WatchEvent> {
        match self.receiver.try_recv() {
            Ok(Ok(event)) => self.process_event(event),
            Ok(Err(e)) => {
                tracing::warn!("Watch error: {:?}", e);
                None
            }
            Err(_) => None,
        }
    }

    /// Wait for the next file system event.
    ///
    /// This is a blocking call that waits until an event is available.
    pub fn wait(&self) -> Option<WatchEvent> {
        match self.receiver.recv() {
            Ok(Ok(event)) => self.process_event(event),
            Ok(Err(e)) => {
                tracing::warn!("Watch error: {:?}", e);
                None
            }
            Err(_) => None,
        }
    }

    /// Wait for an event with a timeout.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<WatchEvent> {
        match self.receiver.recv_timeout(timeout) {
            Ok(Ok(event)) => self.process_event(event),
            Ok(Err(e)) => {
                tracing::warn!("Watch error: {:?}", e);
                None
            }
            Err(_) => None,
        }
    }

    /// Process a raw notify event into our WatchEvent type.
    fn process_event(&self, event: Event) -> Option<WatchEvent> {
        process_event_filtered(event)
    }
}

/// Check if an event kind represents actual content changes worth processing.
/// Filters out noisy events like metadata-only changes and access events.
fn is_meaningful_event(kind: &EventKind) -> bool {
    match kind {
        // Metadata-only changes (permissions, timestamps) - no content change
        EventKind::Modify(ModifyKind::Metadata(_)) => false,

        // "Any" data change is often spurious with no real modification
        EventKind::Modify(ModifyKind::Data(DataChange::Any)) => false,

        // Access events (open, close, read) - not actual modifications
        EventKind::Access(_) => false,

        // Other events that don't indicate content changes
        EventKind::Other => false,

        // All other events are potentially meaningful
        _ => true,
    }
}

/// Process a raw notify event into our WatchEvent type.
/// This is extracted as a free function to make it testable without a FileWatcher instance.
fn process_event_filtered(event: Event) -> Option<WatchEvent> {
    // Filter out noisy events before any other processing
    if !is_meaningful_event(&event.kind) {
        return None;
    }

    // Only process supported source files
    let paths: Vec<_> = event
        .paths
        .into_iter()
        .filter(|p| is_supported_file(p))
        .collect();

    if paths.is_empty() {
        return None;
    }

    match event.kind {
        EventKind::Create(_) => paths.into_iter().next().map(WatchEvent::Created),
        EventKind::Modify(_) => paths.into_iter().next().map(WatchEvent::Modified),
        EventKind::Remove(_) => paths.into_iter().next().map(WatchEvent::Deleted),
        EventKind::Any => paths.into_iter().next().map(WatchEvent::Modified),
        _ => None,
    }
}

/// Default debounce duration for the DebouncedFileWatcher
pub const DEFAULT_DEBOUNCE_DURATION: Duration = Duration::from_millis(200);

/// A debounced file system watcher that reduces duplicate events.
///
/// This is the recommended watcher implementation. It uses `notify-debouncer-full`
/// which provides:
/// - Event debouncing (multiple rapid events are merged)
/// - Proper rename tracking using file IDs
/// - Deduplication of create/modify events
///
/// # Example
/// ```ignore
/// let mut watcher = DebouncedFileWatcher::new(&root, Duration::from_millis(200))?;
/// watcher.start()?;
///
/// loop {
///     for event in watcher.poll_events() {
///         match event {
///             WatchEvent::Modified(path) => update_index(&path),
///             WatchEvent::Deleted(path) => remove_from_index(&path),
///             _ => {}
///         }
///     }
///     std::thread::sleep(Duration::from_millis(100));
/// }
/// ```
pub struct DebouncedFileWatcher {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    receiver: std::sync::mpsc::Receiver<DebounceEventResult>,
    root: PathBuf,
}

impl DebouncedFileWatcher {
    /// Create a new debounced file watcher.
    ///
    /// # Arguments
    /// * `root` - The root directory to watch
    /// * `debounce_duration` - How long to wait before emitting events (recommended: 200ms)
    pub fn new(root: &Path, debounce_duration: Duration) -> Result<Self, notify::Error> {
        let (tx, rx) = std::sync::mpsc::channel();

        let debouncer = new_debouncer(
            debounce_duration,
            None, // Use default tick rate
            move |result: DebounceEventResult| {
                let _ = tx.send(result);
            },
        )?;

        Ok(Self {
            debouncer,
            receiver: rx,
            root: root.to_path_buf(),
        })
    }

    /// Create a new debounced file watcher with default debounce duration (200ms).
    pub fn with_defaults(root: &Path) -> Result<Self, notify::Error> {
        Self::new(root, DEFAULT_DEBOUNCE_DURATION)
    }

    /// Start watching the root directory.
    pub fn start(&mut self) -> Result<(), notify::Error> {
        self.debouncer
            .watch(&self.root, RecursiveMode::Recursive)
            .map_err(|e| notify::Error::generic(&e.to_string()))
    }

    /// Stop watching the root directory.
    pub fn stop(&mut self) -> Result<(), notify::Error> {
        self.debouncer
            .unwatch(&self.root)
            .map_err(|e| notify::Error::generic(&e.to_string()))
    }

    /// Poll for debounced events, returning all available events.
    ///
    /// This is non-blocking and returns an empty vec if no events are available.
    pub fn poll_events(&self) -> Vec<WatchEvent> {
        let mut events = Vec::new();

        // Drain all available results from the channel
        while let Ok(result) = self.receiver.try_recv() {
            match result {
                Ok(debounced_events) => {
                    for debounced_event in debounced_events {
                        if let Some(watch_event) = self.process_debounced_event(debounced_event) {
                            events.push(watch_event);
                        }
                    }
                }
                Err(errors) => {
                    for error in errors {
                        tracing::warn!("Debounced watch error: {:?}", error);
                    }
                }
            }
        }

        events
    }

    /// Wait for events with a timeout, returning all events that arrive.
    pub fn wait_timeout(&self, timeout: Duration) -> Vec<WatchEvent> {
        let mut events = Vec::new();

        // First, try to receive with timeout
        match self.receiver.recv_timeout(timeout) {
            Ok(result) => {
                if let Ok(debounced_events) = result {
                    for debounced_event in debounced_events {
                        if let Some(watch_event) = self.process_debounced_event(debounced_event) {
                            events.push(watch_event);
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // No events within timeout, return empty
                return events;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Channel disconnected
                return events;
            }
        }

        // Drain any additional events that arrived
        events.extend(self.poll_events());
        events
    }

    /// Process a debounced event into our WatchEvent type.
    fn process_debounced_event(&self, event: DebouncedEvent) -> Option<WatchEvent> {
        // Filter out noisy events
        if !is_meaningful_event(&event.kind) {
            return None;
        }

        // Get the first supported file path from the event
        let path = event.paths.iter().find(|p| is_supported_file(p))?.clone();

        match event.kind {
            EventKind::Create(_) => Some(WatchEvent::Created(path)),
            EventKind::Modify(_) => Some(WatchEvent::Modified(path)),
            EventKind::Remove(_) => Some(WatchEvent::Deleted(path)),
            EventKind::Any => Some(WatchEvent::Modified(path)),
            _ => None,
        }
    }
}

/// Check if a path is a supported source file.
/// Supported: C, C++, C#, F#, Go, Java, JavaScript, Kotlin, Objective-C, PHP, Python, Ruby, Rust, Swift, TypeScript.
pub fn is_supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext,
                // C
                "c" | "h"
                    // C++
                    | "cpp"
                    | "cc"
                    | "cxx"
                    | "hpp"
                    | "hxx"
                    | "hh"
                    // C#
                    | "cs"
                    // F#
                    | "fs"
                    | "fsi"
                    | "fsx"
                    // Go
                    | "go"
                    // Java
                    | "java"
                    // JavaScript
                    | "js"
                    | "jsx"
                    | "mjs"
                    | "cjs"
                    // Kotlin
                    | "kt"
                    | "kts"
                    // Objective-C
                    | "m"
                    | "mm"
                    // PHP
                    | "php"
                    // Python
                    | "py"
                    | "pyi"
                    // Ruby
                    | "rb"
                    // Rust
                    | "rs"
                    // Swift
                    | "swift"
                    // TypeScript
                    | "ts"
                    | "tsx"
            )
        })
        .unwrap_or(false)
}

/// Find all supported source files in a directory tree (uses default exclusions).
pub fn find_source_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    find_source_files_with_exclusions(root, crate::config::DEFAULT_EXCLUDE_DIRS)
}

/// Find all supported source files respecting .gitignore and custom exclusions.
///
/// This is the preferred method for indexing as it:
/// - Respects .gitignore files (including nested ones) when `respect_gitignore` is true
/// - Respects global gitignore (~/.gitignore)
/// - Respects .git/info/exclude
/// - Applies custom directory exclusions on top
pub fn find_source_files_with_exclusions(
    root: &Path,
    exclude_dirs: &[&str],
) -> std::io::Result<Vec<PathBuf>> {
    find_source_files_with_config(root, exclude_dirs, true)
}

/// Find all supported source files with full configuration control.
///
/// # Arguments
/// * `root` - Root directory to search
/// * `exclude_dirs` - Additional directories to exclude
/// * `respect_gitignore` - Whether to respect .gitignore files
pub fn find_source_files_with_config(
    root: &Path,
    exclude_dirs: &[&str],
    respect_gitignore: bool,
) -> std::io::Result<Vec<PathBuf>> {
    use ignore::overrides::OverrideBuilder;
    use ignore::WalkBuilder;

    let mut files = Vec::new();

    // Build overrides for custom exclusions (these take precedence)
    let mut override_builder = OverrideBuilder::new(root);
    for dir in exclude_dirs {
        // Exclude pattern: !dir/ means "do not include this directory"
        let pattern = format!("!{}/", dir);
        if let Err(e) = override_builder.add(&pattern) {
            tracing::warn!("Invalid exclude pattern '{}': {}", pattern, e);
        }
    }
    let overrides = match override_builder.build() {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("Failed to build overrides: {}", e);
            // Fall back to empty overrides - continue without custom exclusions
            OverrideBuilder::new(root)
                .build()
                .expect("empty override should succeed")
        }
    };

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true) // Skip hidden files/dirs (like .git)
        .git_ignore(respect_gitignore) // Respect .gitignore
        .git_global(respect_gitignore) // Respect global gitignore
        .git_exclude(respect_gitignore) // Respect .git/info/exclude
        .require_git(false) // Still work in non-git directories
        .ignore(respect_gitignore) // Respect .ignore files
        .parents(respect_gitignore) // Check parent directories for ignore files
        .overrides(overrides); // Apply custom exclusions

    for entry in builder.build() {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.is_file() && is_supported_file(path) {
                    files.push(path.to_path_buf());
                }
            }
            Err(err) => {
                tracing::warn!("Error walking directory: {}", err);
            }
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, MetadataKind, RemoveKind};

    /// Helper to create a test event with the given kind and paths
    fn make_event(kind: EventKind, paths: Vec<PathBuf>) -> Event {
        Event {
            kind,
            paths,
            attrs: Default::default(),
        }
    }

    #[test]
    fn test_is_supported_file() {
        // C
        assert!(is_supported_file(Path::new("test.c")));
        assert!(is_supported_file(Path::new("test.h")));
        // C++
        assert!(is_supported_file(Path::new("test.cpp")));
        assert!(is_supported_file(Path::new("test.cc")));
        assert!(is_supported_file(Path::new("test.cxx")));
        assert!(is_supported_file(Path::new("test.hpp")));
        assert!(is_supported_file(Path::new("test.hxx")));
        assert!(is_supported_file(Path::new("test.hh")));
        // C#
        assert!(is_supported_file(Path::new("test.cs")));
        // F#
        assert!(is_supported_file(Path::new("test.fs")));
        assert!(is_supported_file(Path::new("test.fsi")));
        assert!(is_supported_file(Path::new("test.fsx")));
        // Go
        assert!(is_supported_file(Path::new("test.go")));
        // Java
        assert!(is_supported_file(Path::new("test.java")));
        // JavaScript
        assert!(is_supported_file(Path::new("test.js")));
        assert!(is_supported_file(Path::new("test.jsx")));
        assert!(is_supported_file(Path::new("test.mjs")));
        assert!(is_supported_file(Path::new("test.cjs")));
        // Kotlin
        assert!(is_supported_file(Path::new("test.kt")));
        assert!(is_supported_file(Path::new("test.kts")));
        // Objective-C
        assert!(is_supported_file(Path::new("test.m")));
        assert!(is_supported_file(Path::new("test.mm")));
        // Swift
        assert!(is_supported_file(Path::new("test.swift")));
        // Ruby
        assert!(is_supported_file(Path::new("test.rb")));
        // Rust
        assert!(is_supported_file(Path::new("test.rs")));
        // TypeScript
        assert!(is_supported_file(Path::new("test.ts")));
        assert!(is_supported_file(Path::new("test.tsx")));
        // PHP
        assert!(is_supported_file(Path::new("test.php")));
        // Paths
        assert!(is_supported_file(Path::new("/path/to/Module.fs")));
        assert!(is_supported_file(Path::new("/path/to/main.go")));
        assert!(is_supported_file(Path::new("/path/to/app.ts")));

        assert!(!is_supported_file(Path::new("test.txt")));
        assert!(!is_supported_file(Path::new("test")));
    }

    #[test]
    fn test_find_source_files() {
        // This test would need a temp directory with test files
        // For now, just verify the function exists and returns Ok
        let result = find_source_files(Path::new("."));
        assert!(result.is_ok());
    }

    // ==================== Event Filtering Tests ====================

    #[test]
    fn test_filters_metadata_only_changes() {
        // Metadata changes (permissions, timestamps) should be filtered out
        let event = make_event(
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());

        // Specifically permission changes
        let event = make_event(
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Permissions)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());

        // Ownership changes
        let event = make_event(
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Ownership)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());

        // Access time changes
        let event = make_event(
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());

        // Write time changes (without content change)
        let event = make_event(
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());
    }

    #[test]
    fn test_filters_spurious_any_data_change() {
        // DataChange::Any is often spurious with no real content modification
        let event = make_event(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());
    }

    #[test]
    fn test_filters_access_events() {
        // File open events
        let event = make_event(
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());

        // File close events (even close after write - the modify event handles the actual change)
        let event = make_event(
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());

        // Read access
        let event = make_event(
            EventKind::Access(AccessKind::Read),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());

        // Any access
        let event = make_event(
            EventKind::Access(AccessKind::Any),
            vec![PathBuf::from("test.rs")],
        );
        assert!(process_event_filtered(event).is_none());
    }

    #[test]
    fn test_filters_other_events() {
        // EventKind::Other should be filtered
        let event = make_event(EventKind::Other, vec![PathBuf::from("test.rs")]);
        assert!(process_event_filtered(event).is_none());
    }

    #[test]
    fn test_allows_create_events() {
        let event = make_event(
            EventKind::Create(CreateKind::File),
            vec![PathBuf::from("test.rs")],
        );
        let result = process_event_filtered(event);
        assert!(matches!(result, Some(WatchEvent::Created(_))));
    }

    #[test]
    fn test_allows_real_modify_events() {
        // Content modification should be allowed
        let event = make_event(
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            vec![PathBuf::from("test.rs")],
        );
        let result = process_event_filtered(event);
        assert!(matches!(result, Some(WatchEvent::Modified(_))));

        // Size change should be allowed
        let event = make_event(
            EventKind::Modify(ModifyKind::Data(DataChange::Size)),
            vec![PathBuf::from("test.rs")],
        );
        let result = process_event_filtered(event);
        assert!(matches!(result, Some(WatchEvent::Modified(_))));

        // Generic modify (ModifyKind::Any) should be allowed
        let event = make_event(
            EventKind::Modify(ModifyKind::Any),
            vec![PathBuf::from("test.rs")],
        );
        let result = process_event_filtered(event);
        assert!(matches!(result, Some(WatchEvent::Modified(_))));
    }

    #[test]
    fn test_allows_remove_events() {
        let event = make_event(
            EventKind::Remove(RemoveKind::File),
            vec![PathBuf::from("test.rs")],
        );
        let result = process_event_filtered(event);
        assert!(matches!(result, Some(WatchEvent::Deleted(_))));
    }

    #[test]
    fn test_still_filters_unsupported_extensions() {
        // Even meaningful events should be filtered if extension is unsupported
        let event = make_event(
            EventKind::Create(CreateKind::File),
            vec![PathBuf::from("test.txt")],
        );
        assert!(process_event_filtered(event).is_none());

        let event = make_event(
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            vec![PathBuf::from("readme.md")],
        );
        assert!(process_event_filtered(event).is_none());
    }

    #[test]
    fn test_is_meaningful_event_classification() {
        // These should be filtered (not meaningful)
        assert!(!is_meaningful_event(&EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::Any)
        )));
        assert!(!is_meaningful_event(&EventKind::Modify(ModifyKind::Data(
            DataChange::Any
        ))));
        assert!(!is_meaningful_event(&EventKind::Access(AccessKind::Any)));
        assert!(!is_meaningful_event(&EventKind::Other));

        // These should be allowed (meaningful)
        assert!(is_meaningful_event(&EventKind::Create(CreateKind::File)));
        assert!(is_meaningful_event(&EventKind::Modify(ModifyKind::Data(
            DataChange::Content
        ))));
        assert!(is_meaningful_event(&EventKind::Modify(ModifyKind::Any)));
        assert!(is_meaningful_event(&EventKind::Remove(RemoveKind::File)));
        assert!(is_meaningful_event(&EventKind::Any));
    }

    // ==================== DebouncedFileWatcher Tests ====================

    #[test]
    fn test_debounced_watcher_creation() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = DebouncedFileWatcher::new(dir.path(), Duration::from_millis(100));
        assert!(watcher.is_ok());
    }

    #[test]
    fn test_debounced_watcher_with_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = DebouncedFileWatcher::with_defaults(dir.path());
        assert!(watcher.is_ok());
    }

    #[test]
    fn test_debounced_watcher_start_stop() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut watcher = DebouncedFileWatcher::with_defaults(dir.path()).unwrap();

        // Should start without error
        assert!(watcher.start().is_ok());

        // Should stop without error
        assert!(watcher.stop().is_ok());
    }

    #[test]
    fn test_debounced_watcher_poll_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = DebouncedFileWatcher::with_defaults(dir.path()).unwrap();

        // Polling without any changes should return empty
        let events = watcher.poll_events();
        assert!(events.is_empty());
    }

    #[test]
    fn test_debounced_watcher_wait_timeout_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = DebouncedFileWatcher::with_defaults(dir.path()).unwrap();

        // Waiting with timeout should return empty when no events
        let events = watcher.wait_timeout(Duration::from_millis(50));
        assert!(events.is_empty());
    }

    #[test]
    fn test_default_debounce_duration() {
        assert_eq!(DEFAULT_DEBOUNCE_DURATION, Duration::from_millis(200));
    }

    // ==================== Integration Tests ====================

    #[test]
    fn test_debounced_watcher_detects_file_changes() {
        use std::fs::File;
        use std::io::Write;

        let dir = tempfile::TempDir::new().unwrap();
        let mut watcher =
            DebouncedFileWatcher::new(dir.path(), Duration::from_millis(100)).unwrap();
        watcher.start().unwrap();

        // Create a Rust file (supported extension)
        let test_file = dir.path().join("test.rs");
        {
            let mut file = File::create(&test_file).unwrap();
            writeln!(file, "fn main() {{}}").unwrap();
        }

        // Wait for debounce + some margin
        std::thread::sleep(Duration::from_millis(300));

        let events = watcher.poll_events();
        // Should have at least one event for the created file
        // Note: On macOS, paths may differ due to /var -> /private/var symlink
        let test_file_canonical = test_file.canonicalize().unwrap_or(test_file.clone());
        assert!(
            events.iter().any(|e| {
                match e {
                    WatchEvent::Created(p) | WatchEvent::Modified(p) => {
                        let p_canonical = p.canonicalize().unwrap_or(p.clone());
                        p_canonical == test_file_canonical
                    }
                    _ => false,
                }
            }),
            "Expected event for {:?}, got {:?}",
            test_file,
            events
        );
    }

    #[test]
    fn test_debounced_watcher_ignores_unsupported_files() {
        use std::fs::File;
        use std::io::Write;

        let dir = tempfile::TempDir::new().unwrap();
        let mut watcher =
            DebouncedFileWatcher::new(dir.path(), Duration::from_millis(100)).unwrap();
        watcher.start().unwrap();

        // Create a text file (unsupported extension)
        let test_file = dir.path().join("readme.txt");
        {
            let mut file = File::create(&test_file).unwrap();
            writeln!(file, "Hello").unwrap();
        }

        // Wait for debounce + some margin
        std::thread::sleep(Duration::from_millis(300));

        let events = watcher.poll_events();
        // Should NOT have any events for unsupported file
        assert!(
            !events.iter().any(|e| match e {
                WatchEvent::Created(p) | WatchEvent::Modified(p) | WatchEvent::Deleted(p) => {
                    p == &test_file
                }
                WatchEvent::Renamed(old, new) => old == &test_file || new == &test_file,
            }),
            "Should not have events for unsupported file, got {:?}",
            events
        );
    }

    #[test]
    fn test_debounced_watcher_coalesces_rapid_changes() {
        use std::fs::File;
        use std::io::Write;

        let dir = tempfile::TempDir::new().unwrap();
        // Use longer debounce to ensure coalescing
        let mut watcher =
            DebouncedFileWatcher::new(dir.path(), Duration::from_millis(200)).unwrap();
        watcher.start().unwrap();

        let test_file = dir.path().join("rapid.rs");

        // Make many rapid changes to the same file
        for i in 0..5 {
            let mut file = File::create(&test_file).unwrap();
            writeln!(file, "fn version{}() {{}}", i).unwrap();
            std::thread::sleep(Duration::from_millis(20)); // 20ms between writes
        }

        // Wait for debounce window to close
        std::thread::sleep(Duration::from_millis(400));

        let events = watcher.poll_events();

        // Count events for our file - debouncing should reduce the count
        let file_events: Vec<_> = events
            .iter()
            .filter(|e| match e {
                WatchEvent::Created(p) | WatchEvent::Modified(p) => p == &test_file,
                _ => false,
            })
            .collect();

        // With debouncing, we should have fewer events than the 5 writes we made
        // Exact count depends on timing, but should be significantly less than 5
        assert!(
            file_events.len() <= 3,
            "Expected debouncing to reduce events, got {} events: {:?}",
            file_events.len(),
            file_events
        );
    }
}
