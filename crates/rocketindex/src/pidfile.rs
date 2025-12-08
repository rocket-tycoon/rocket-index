//! PID file coordination for watch mode.
//!
//! This module provides cross-platform process coordination using PID files,
//! replacing the fragile pgrep-based approach.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Name of the PID file within the .rocketindex directory
pub const PID_FILE_NAME: &str = "watch.pid";

/// Errors that can occur during PID file operations
#[derive(Debug, thiserror::Error)]
pub enum PidFileError {
    #[error("Watch mode is already running (pid {0})")]
    AlreadyRunning(u32),

    #[error("Failed to create PID file: {0}")]
    CreateFailed(#[from] std::io::Error),

    #[error("Invalid PID file contents")]
    InvalidContents,
}

/// A guard that holds a PID file lock.
/// The PID file is automatically cleaned up when this guard is dropped.
pub struct PidFileGuard {
    path: PathBuf,
}

impl PidFileGuard {
    /// Get the path to the PID file
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PidFileGuard {
    fn drop(&mut self) {
        // Best-effort cleanup of PID file
        let _ = fs::remove_file(&self.path);
    }
}

/// Get the path to the PID file for a given root directory.
pub fn pid_file_path(root: &Path) -> PathBuf {
    root.join(".rocketindex").join(PID_FILE_NAME)
}

/// Attempt to acquire the watch lock by creating a PID file.
///
/// Returns a guard that will clean up the PID file when dropped.
/// Returns an error if another watch process is already running.
pub fn acquire_watch_lock(root: &Path) -> Result<PidFileGuard, PidFileError> {
    let pid_path = pid_file_path(root);

    // Ensure .rocketindex directory exists
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Check if another process is running
    if let Some(existing_pid) = read_pid_file(&pid_path)? {
        if is_process_alive(existing_pid) {
            return Err(PidFileError::AlreadyRunning(existing_pid));
        }
        // Stale PID file - process is dead, remove it
        fs::remove_file(&pid_path)?;
    }

    // Create new PID file with our PID
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true) // Fail if file exists (race condition protection)
        .open(&pid_path)
        .or_else(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                // Race condition: another process created the file between our check and create
                // Re-read and check if that process is alive
                if let Some(pid) = read_pid_file(&pid_path).ok().flatten() {
                    if is_process_alive(pid) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            format!("Watch mode is already running (pid {})", pid),
                        ));
                    }
                }
                // Other process died, try to remove and recreate
                let _ = fs::remove_file(&pid_path);
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&pid_path)
            } else {
                Err(e)
            }
        })?;

    writeln!(file, "{}", std::process::id())?;

    Ok(PidFileGuard { path: pid_path })
}

/// Check if a watch process is running for the given directory.
/// Returns the PID if a live watch process is found.
pub fn find_watch_process(root: &Path) -> Option<u32> {
    let pid_path = pid_file_path(root);

    match read_pid_file(&pid_path) {
        Ok(Some(pid)) if is_process_alive(pid) => Some(pid),
        _ => None,
    }
}

/// Read a PID from a PID file.
fn read_pid_file(path: &Path) -> Result<Option<u32>, PidFileError> {
    if !path.exists() {
        return Ok(None);
    }

    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let pid = contents
        .trim()
        .parse::<u32>()
        .map_err(|_| PidFileError::InvalidContents)?;

    Ok(Some(pid))
}

/// Check if a process with the given PID is still alive.
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    // On Unix, sending signal 0 checks if process exists without actually signaling
    // SAFETY: This is a standard Unix API call
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use std::ptr::null_mut;

    // PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    unsafe {
        let handle = windows_sys::Win32::System::Threading::OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            0, // bInheritHandle = FALSE
            pid,
        );

        if handle.is_null() {
            return false;
        }

        let mut exit_code: u32 = 0;
        let result = windows_sys::Win32::System::Threading::GetExitCodeProcess(
            handle,
            &mut exit_code as *mut u32,
        );

        windows_sys::Win32::Foundation::CloseHandle(handle);

        // STILL_ACTIVE = 259
        result != 0 && exit_code == 259
    }
}

#[cfg(not(any(unix, windows)))]
fn is_process_alive(_pid: u32) -> bool {
    // On unknown platforms, assume the process might be alive
    // This is conservative - it may report false positives
    true
}

/// Remove a stale PID file if the process is no longer running.
/// Returns true if a stale file was removed.
pub fn cleanup_stale_pidfile(root: &Path) -> bool {
    let pid_path = pid_file_path(root);

    if let Ok(Some(pid)) = read_pid_file(&pid_path) {
        if !is_process_alive(pid) {
            return fs::remove_file(&pid_path).is_ok();
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".rocketindex")).unwrap();
        dir
    }

    #[test]
    fn test_pid_file_path() {
        let path = pid_file_path(Path::new("/some/project"));
        assert_eq!(path, PathBuf::from("/some/project/.rocketindex/watch.pid"));
    }

    #[test]
    fn test_acquire_lock_creates_pid_file() {
        let dir = setup_test_dir();
        let guard = acquire_watch_lock(dir.path()).unwrap();

        let pid_path = pid_file_path(dir.path());
        assert!(pid_path.exists());

        // Verify PID file contains our PID
        let contents = fs::read_to_string(&pid_path).unwrap();
        let stored_pid: u32 = contents.trim().parse().unwrap();
        assert_eq!(stored_pid, std::process::id());

        drop(guard);
    }

    #[test]
    fn test_guard_cleans_up_on_drop() {
        let dir = setup_test_dir();
        let pid_path = pid_file_path(dir.path());

        {
            let _guard = acquire_watch_lock(dir.path()).unwrap();
            assert!(pid_path.exists());
        }

        // After guard is dropped, PID file should be removed
        assert!(!pid_path.exists());
    }

    #[test]
    fn test_find_watch_process_returns_none_when_no_file() {
        let dir = setup_test_dir();
        assert!(find_watch_process(dir.path()).is_none());
    }

    #[test]
    fn test_find_watch_process_returns_pid_when_alive() {
        let dir = setup_test_dir();
        let _guard = acquire_watch_lock(dir.path()).unwrap();

        // Our own process should be detected as alive
        let found = find_watch_process(dir.path());
        assert_eq!(found, Some(std::process::id()));
    }

    #[test]
    fn test_find_watch_process_returns_none_for_dead_process() {
        let dir = setup_test_dir();
        let pid_path = pid_file_path(dir.path());

        // Write a PID that definitely doesn't exist (very high number)
        fs::write(&pid_path, "999999999\n").unwrap();

        // Should return None because process is dead
        // Note: This test might fail if PID 999999999 happens to exist
        assert!(find_watch_process(dir.path()).is_none());
    }

    #[test]
    fn test_cleanup_stale_pidfile_removes_dead_process() {
        let dir = setup_test_dir();
        let pid_path = pid_file_path(dir.path());

        // Write a stale PID file
        fs::write(&pid_path, "999999999\n").unwrap();
        assert!(pid_path.exists());

        // Cleanup should remove it
        let removed = cleanup_stale_pidfile(dir.path());
        assert!(removed);
        assert!(!pid_path.exists());
    }

    #[test]
    fn test_cleanup_stale_pidfile_preserves_live_process() {
        let dir = setup_test_dir();
        let _guard = acquire_watch_lock(dir.path()).unwrap();

        // Cleanup should NOT remove our own PID file
        let removed = cleanup_stale_pidfile(dir.path());
        assert!(!removed);
        assert!(pid_file_path(dir.path()).exists());
    }

    #[test]
    fn test_acquire_lock_cleans_stale_and_succeeds() {
        let dir = setup_test_dir();
        let pid_path = pid_file_path(dir.path());

        // Write a stale PID file
        fs::write(&pid_path, "999999999\n").unwrap();

        // Should succeed by cleaning up the stale file
        let guard = acquire_watch_lock(dir.path());
        assert!(guard.is_ok());

        // Verify our PID is now in the file
        let contents = fs::read_to_string(&pid_path).unwrap();
        let stored_pid: u32 = contents.trim().parse().unwrap();
        assert_eq!(stored_pid, std::process::id());
    }

    #[test]
    fn test_acquire_lock_fails_when_already_running() {
        let dir = setup_test_dir();

        // First acquisition should succeed
        let guard1 = acquire_watch_lock(dir.path()).unwrap();

        // Second acquisition should fail
        let result = acquire_watch_lock(dir.path());
        assert!(matches!(result, Err(PidFileError::AlreadyRunning(_))));

        drop(guard1);
    }

    #[test]
    fn test_creates_rocketindex_directory_if_missing() {
        let dir = TempDir::new().unwrap();
        // Don't create .rocketindex directory

        let guard = acquire_watch_lock(dir.path());
        assert!(guard.is_ok());
        assert!(dir.path().join(".rocketindex").exists());
    }

    #[test]
    fn test_read_pid_file_handles_invalid_contents() {
        let dir = setup_test_dir();
        let pid_path = pid_file_path(dir.path());

        // Write invalid content
        fs::write(&pid_path, "not-a-number\n").unwrap();

        let result = read_pid_file(&pid_path);
        assert!(matches!(result, Err(PidFileError::InvalidContents)));
    }

    #[test]
    fn test_is_process_alive_current_process() {
        // Our own process should definitely be alive
        assert!(is_process_alive(std::process::id()));
    }

    #[test]
    fn test_is_process_alive_nonexistent_process() {
        // A very high PID is unlikely to exist
        assert!(!is_process_alive(999999999));
    }
}
