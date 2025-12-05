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

/// Check if a path is a supported source file (F# or Ruby).
pub fn is_supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext, "fs" | "fsi" | "fsx" | "rb"))
        .unwrap_or(false)
}

/// Find all supported source files in a directory tree (uses default exclusions).
pub fn find_source_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    find_source_files_with_exclusions(root, crate::config::DEFAULT_EXCLUDE_DIRS)
}

/// Find all supported source files in a directory tree with custom exclusions.
pub fn find_source_files_with_exclusions(
    root: &Path,
    exclude_dirs: &[&str],
) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    find_source_files_recursive(root, &mut files, exclude_dirs)?;
    Ok(files)
}

fn find_source_files_recursive(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    exclude_dirs: &[&str],
) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Skip hidden directories and excluded directories
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !dir_name.starts_with('.') && !exclude_dirs.contains(&dir_name) {
                find_source_files_recursive(&path, files, exclude_dirs)?;
            }
        } else if is_supported_file(&path) {
            files.push(path);
        }
    }

    Ok(())
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
        assert!(is_supported_file(Path::new("/path/to/Module.fs")));

        assert!(!is_supported_file(Path::new("test.cs")));
        assert!(!is_supported_file(Path::new("test.rs")));
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
