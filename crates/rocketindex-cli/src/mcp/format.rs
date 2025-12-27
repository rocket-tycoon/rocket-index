//! Output formatting utilities for MCP responses.
//!
//! This module provides utilities to reduce token count in MCP responses
//! by using relative paths and compact JSON formatting.

use std::path::Path;

/// Converts an absolute file path to a path relative to the project root.
///
/// If the path is not under the project root, returns the original path unchanged.
/// Handles symlinks by canonicalizing both paths before comparison.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// let abs = Path::new("/home/user/project/src/main.rs");
/// let root = Path::new("/home/user/project");
/// let rel = to_relative_path(abs, root);
/// assert_eq!(rel, "src/main.rs");
/// ```
pub fn to_relative_path(file_path: &Path, project_root: &Path) -> String {
    // Try direct strip_prefix first (fast path)
    if let Ok(rel) = file_path.strip_prefix(project_root) {
        return rel.display().to_string();
    }

    // If that fails, try canonicalizing both paths to handle symlinks
    // This is common on macOS where /var -> /private/var
    if let (Ok(canonical_file), Ok(canonical_root)) =
        (file_path.canonicalize(), project_root.canonicalize())
    {
        if let Ok(rel) = canonical_file.strip_prefix(&canonical_root) {
            return rel.display().to_string();
        }
    }

    // Fall back to original path
    file_path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_relative_path_simple() {
        let root = PathBuf::from("/home/user/project");
        let file = PathBuf::from("/home/user/project/src/main.rs");

        let result = to_relative_path(&file, &root);
        assert_eq!(result, "src/main.rs");
    }

    #[test]
    fn test_relative_path_nested() {
        let root = PathBuf::from("/home/user/project");
        let file = PathBuf::from("/home/user/project/src/lib/utils/helper.rs");

        let result = to_relative_path(&file, &root);
        assert_eq!(result, "src/lib/utils/helper.rs");
    }

    #[test]
    fn test_relative_path_at_root() {
        let root = PathBuf::from("/home/user/project");
        let file = PathBuf::from("/home/user/project/Cargo.toml");

        let result = to_relative_path(&file, &root);
        assert_eq!(result, "Cargo.toml");
    }

    #[test]
    fn test_relative_path_outside_project() {
        let root = PathBuf::from("/home/user/project");
        let file = PathBuf::from("/home/user/other/file.rs");

        let result = to_relative_path(&file, &root);
        // Should return the original absolute path
        assert_eq!(result, "/home/user/other/file.rs");
    }

    #[test]
    fn test_relative_path_same_as_root() {
        let root = PathBuf::from("/home/user/project");
        let file = PathBuf::from("/home/user/project");

        let result = to_relative_path(&file, &root);
        // Empty string for exact match
        assert_eq!(result, "");
    }

    #[test]
    fn test_relative_path_with_trailing_slash() {
        let root = PathBuf::from("/home/user/project/");
        let file = PathBuf::from("/home/user/project/src/main.rs");

        let result = to_relative_path(&file, &root);
        assert_eq!(result, "src/main.rs");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_relative_path_macos_style() {
        let root = PathBuf::from("/Users/alastair/Documents/work/project");
        let file = PathBuf::from("/Users/alastair/Documents/work/project/src/lib.rs");

        let result = to_relative_path(&file, &root);
        assert_eq!(result, "src/lib.rs");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_relative_path_windows_style() {
        let root = PathBuf::from("C:\\Users\\user\\project");
        let file = PathBuf::from("C:\\Users\\user\\project\\src\\main.rs");

        let result = to_relative_path(&file, &root);
        assert_eq!(result, "src\\main.rs");
    }
}
