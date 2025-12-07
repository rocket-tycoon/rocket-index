//! File system watcher for incremental index updates.
//!
//! This module provides functionality to watch F# source files for changes
//! and trigger incremental re-indexing when files are modified.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

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
        // Only process F# source files
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
}

/// Check if a path is a supported source file (F#, Ruby, Python, Rust, or Go).
pub fn is_supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext,
                "fs" | "fsi" | "fsx" | "rb" | "py" | "pyi" | "rs" | "go"
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

    #[test]
    fn test_is_supported_file() {
        assert!(is_supported_file(Path::new("test.fs")));
        assert!(is_supported_file(Path::new("test.fsi")));
        assert!(is_supported_file(Path::new("test.fsx")));
        assert!(is_supported_file(Path::new("test.rb")));
        assert!(is_supported_file(Path::new("test.rs")));
        assert!(is_supported_file(Path::new("test.go")));
        assert!(is_supported_file(Path::new("/path/to/Module.fs")));
        assert!(is_supported_file(Path::new("/path/to/main.go")));

        assert!(!is_supported_file(Path::new("test.cs")));
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
}
