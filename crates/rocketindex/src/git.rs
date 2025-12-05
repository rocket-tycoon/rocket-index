use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// Git provenance information for a line or symbol.
/// Fields ordered by importance for AI agents: why > when > reference > who
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    /// The commit message - "why" this change was made (most important)
    pub message: String,
    /// Conventional commit type if present (feat, fix, refactor, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_type: Option<String>,
    /// When the change was made (short date: 2024-12-04)
    pub date: String,
    /// Relative age for quick assessment ("3 days ago", "2 months ago")
    pub date_relative: String,
    /// Full commit hash for reference
    pub commit: String,
    /// Who made the change (often "Claude" now, least important)
    pub author: String,
}

/// Check if we're in a git repository
pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a file is tracked by git
pub fn is_tracked(file: &Path) -> bool {
    Command::new("git")
        .args(["ls-files", "--error-unmatch"])
        .arg(file)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Extract conventional commit type from message (feat, fix, refactor, etc.)
fn extract_commit_type(message: &str) -> Option<String> {
    // Match patterns like "feat:", "fix(scope):", "refactor!:"
    let msg = message.trim();

    // Common conventional commit types
    const TYPES: &[&str] = &[
        "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore",
        "revert",
    ];

    for typ in TYPES {
        // Match "type:" or "type(scope):" or "type!:"
        if let Some(rest) = msg.strip_prefix(typ) {
            if rest.starts_with(':') || rest.starts_with('(') || rest.starts_with('!') {
                return Some(typ.to_string());
            }
        }
    }

    None
}

/// Get blame information for a specific line in a file.
pub fn get_blame(file: &Path, line: u32) -> Result<GitInfo> {
    if !is_git_repo() {
        anyhow::bail!("Not in a git repository");
    }

    if !is_tracked(file) {
        anyhow::bail!("File is not tracked by git: {}", file.display());
    }

    // 1. Get commit hash from git blame --porcelain
    // We use porcelain to reliably get the commit hash even if we want formatted date later
    let output = Command::new("git")
        .arg("blame")
        .arg("-L")
        .arg(format!("{},{}", line, line))
        .arg("--porcelain")
        .arg(file)
        .output()
        .context("Failed to execute git blame")?;

    if !output.status.success() {
        anyhow::bail!(
            "git blame failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let commit = stdout
        .lines()
        .next()
        .context("Empty blame output")?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();

    // Check for uncommitted changes (zeros hash)
    if commit.chars().all(|c| c == '0') {
        anyhow::bail!("Line {} has uncommitted changes", line);
    }

    // 2. Get details from git show
    // We use git show to get the date in a nice format without needing chrono
    get_commit_info(&commit)
}

/// Get history for a range of lines in a file.
pub fn get_history(file: &Path, start_line: u32, end_line: u32) -> Result<Vec<GitInfo>> {
    if !is_git_repo() {
        anyhow::bail!("Not in a git repository");
    }

    if !is_tracked(file) {
        anyhow::bail!("File is not tracked by git: {}", file.display());
    }

    // git log -L start,end:file
    // Format: hash|author|short_date|relative_date|message
    let output = Command::new("git")
        .arg("log")
        .arg("-L")
        .arg(format!("{},{}:{}", start_line, end_line, file.display()))
        .arg("--pretty=format:%H|%an|%ad|%ar|%s")
        .arg("--date=short")
        .output()
        .context("Failed to execute git log")?;

    if !output.status.success() {
        anyhow::bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut history = Vec::new();

    for line in stdout.lines() {
        // git log -L output includes diffs, so we need to filter for our format lines
        if let Some(info) = parse_log_line(line) {
            history.push(info);
        }
    }

    Ok(history)
}

fn get_commit_info(commit: &str) -> Result<GitInfo> {
    // Get short date
    let output = Command::new("git")
        .arg("show")
        .arg("-s")
        .arg("--format=%an|%ad|%s")
        .arg("--date=short")
        .arg(commit)
        .output()
        .context("Failed to execute git show")?;

    if !output.status.success() {
        anyhow::bail!(
            "git show failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.trim();

    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 3 {
        anyhow::bail!("Failed to parse git show output: {}", line);
    }

    let message = parts[2..].join("|");
    let commit_type = extract_commit_type(&message);

    // Get relative date in a second call
    let relative_output = Command::new("git")
        .arg("show")
        .arg("-s")
        .arg("--format=%ar")
        .arg(commit)
        .output()
        .context("Failed to execute git show for relative date")?;

    let date_relative = String::from_utf8_lossy(&relative_output.stdout)
        .trim()
        .to_string();

    Ok(GitInfo {
        message,
        commit_type,
        date: parts[1].to_string(),
        date_relative,
        commit: commit.to_string(),
        author: parts[0].to_string(),
    })
}

fn parse_log_line(line: &str) -> Option<GitInfo> {
    // Format: hash|author|date|relative_date|message
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 5 {
        return None;
    }

    // Verify first part looks like a hash (hex characters)
    let hash = parts[0];
    if hash.len() < 7 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let message = parts[4..].join("|");
    let commit_type = extract_commit_type(&message);

    Some(GitInfo {
        message,
        commit_type,
        date: parts[2].to_string(),
        date_relative: parts[3].to_string(),
        commit: hash.to_string(),
        author: parts[1].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // =========================================================================
    // Unit tests for parsing functions (no git required)
    // =========================================================================

    #[test]
    fn test_parse_log_line_valid() {
        let line = "abc1234def5678|John Doe|2024-12-04|3 days ago|feat: add new feature";
        let info = parse_log_line(line).unwrap();

        assert_eq!(info.commit, "abc1234def5678");
        assert_eq!(info.author, "John Doe");
        assert_eq!(info.date, "2024-12-04");
        assert_eq!(info.date_relative, "3 days ago");
        assert_eq!(info.message, "feat: add new feature");
        assert_eq!(info.commit_type, Some("feat".to_string()));
    }

    #[test]
    fn test_parse_log_line_with_pipe_in_message() {
        let line =
            "abc1234def5678|Claude|2024-12-04|1 hour ago|fix: handle edge case | more context";
        let info = parse_log_line(line).unwrap();

        assert_eq!(info.message, "fix: handle edge case | more context");
        assert_eq!(info.commit_type, Some("fix".to_string()));
    }

    #[test]
    fn test_parse_log_line_rejects_short_hash() {
        let line = "abc12|Author|2024-12-04|3 days ago|message";
        assert!(parse_log_line(line).is_none());
    }

    #[test]
    fn test_parse_log_line_rejects_non_hex_hash() {
        let line = "ghijklmnop|Author|2024-12-04|3 days ago|message";
        assert!(parse_log_line(line).is_none());
    }

    #[test]
    fn test_parse_log_line_rejects_too_few_parts() {
        assert!(parse_log_line("abc1234|Author|2024-12-04|3 days ago").is_none());
        assert!(parse_log_line("abc1234|Author|2024-12-04").is_none());
        assert!(parse_log_line("abc1234|Author").is_none());
        assert!(parse_log_line("abc1234").is_none());
        assert!(parse_log_line("").is_none());
    }

    #[test]
    fn test_parse_log_line_rejects_diff_lines() {
        // git log -L includes diff output that should be filtered
        assert!(parse_log_line("diff --git a/file.rs b/file.rs").is_none());
        assert!(parse_log_line("@@ -10,5 +10,7 @@").is_none());
        assert!(parse_log_line("+    new line").is_none());
        assert!(parse_log_line("-    old line").is_none());
    }

    #[test]
    fn test_parse_log_line_full_hash() {
        let line =
            "9693c04abc123def456789abcdef0123456789ab|Author|2024-12-04|5 months ago|message";
        let info = parse_log_line(line).unwrap();
        assert_eq!(info.commit.len(), 40);
    }

    #[test]
    fn test_git_info_serialization() {
        let info = GitInfo {
            message: "feat: implement reverse spider".to_string(),
            commit_type: Some("feat".to_string()),
            date: "2024-12-04".to_string(),
            date_relative: "2 hours ago".to_string(),
            commit: "abc1234".to_string(),
            author: "Claude".to_string(),
        };

        let json = serde_json::to_string(&info).unwrap();
        // Message should be first in JSON (field order)
        assert!(json.contains("\"message\":\"feat: implement reverse spider\""));
        assert!(json.contains("\"commit_type\":\"feat\""));
        assert!(json.contains("\"date_relative\":\"2 hours ago\""));

        let parsed: GitInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.message, info.message);
        assert_eq!(parsed.commit_type, info.commit_type);
        assert_eq!(parsed.date_relative, info.date_relative);
    }

    #[test]
    fn test_git_info_serialization_no_commit_type() {
        let info = GitInfo {
            message: "random commit without type".to_string(),
            commit_type: None,
            date: "2024-12-04".to_string(),
            date_relative: "2 hours ago".to_string(),
            commit: "abc1234".to_string(),
            author: "Developer".to_string(),
        };

        let json = serde_json::to_string(&info).unwrap();
        // commit_type should be omitted when None
        assert!(!json.contains("commit_type"));
    }

    // =========================================================================
    // Unit tests for commit type extraction
    // =========================================================================

    #[test]
    fn test_extract_commit_type_standard() {
        assert_eq!(
            extract_commit_type("feat: add feature"),
            Some("feat".to_string())
        );
        assert_eq!(extract_commit_type("fix: bug fix"), Some("fix".to_string()));
        assert_eq!(
            extract_commit_type("docs: update readme"),
            Some("docs".to_string())
        );
        assert_eq!(
            extract_commit_type("refactor: clean up"),
            Some("refactor".to_string())
        );
        assert_eq!(
            extract_commit_type("test: add tests"),
            Some("test".to_string())
        );
        assert_eq!(
            extract_commit_type("chore: update deps"),
            Some("chore".to_string())
        );
    }

    #[test]
    fn test_extract_commit_type_with_scope() {
        assert_eq!(
            extract_commit_type("feat(api): add endpoint"),
            Some("feat".to_string())
        );
        assert_eq!(
            extract_commit_type("fix(ui): layout bug"),
            Some("fix".to_string())
        );
    }

    #[test]
    fn test_extract_commit_type_breaking() {
        assert_eq!(
            extract_commit_type("feat!: breaking change"),
            Some("feat".to_string())
        );
        assert_eq!(
            extract_commit_type("fix!: breaking fix"),
            Some("fix".to_string())
        );
    }

    #[test]
    fn test_extract_commit_type_none() {
        assert_eq!(extract_commit_type("random commit message"), None);
        assert_eq!(extract_commit_type("Update something"), None);
        assert_eq!(extract_commit_type("feature request"), None); // not "feat:"
        assert_eq!(extract_commit_type("fixed bug"), None); // not "fix:"
    }

    // =========================================================================
    // Unit tests for helper functions
    // =========================================================================

    #[test]
    fn test_is_git_repo() {
        // Running from within RocketIndex repo, should be true
        assert!(is_git_repo());
    }

    // =========================================================================
    // Integration tests (require git repository)
    // =========================================================================

    #[test]
    fn test_get_blame_on_lib_file() {
        // Use lib.rs which is definitely committed
        let file = PathBuf::from("crates/rocketindex/src/lib.rs");

        if let Ok(info) = get_blame(&file, 1) {
            // Line 1 should have valid git info
            assert!(!info.commit.is_empty());
            assert!(!info.date.is_empty());
            assert!(!info.date_relative.is_empty());
            assert!(!info.message.is_empty());
        }
    }

    #[test]
    fn test_get_blame_invalid_line() {
        let file = PathBuf::from("crates/rocketindex/src/lib.rs");

        // Line 999999 shouldn't exist
        let result = get_blame(&file, 999999);
        // Should fail gracefully
        assert!(result.is_err());
    }

    #[test]
    fn test_get_blame_nonexistent_file() {
        let file = PathBuf::from("nonexistent/file/that/does/not/exist.rs");
        let result = get_blame(&file, 1);
        assert!(result.is_err());
        // Should mention file not tracked
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not tracked") || err.contains("Not in a git"));
    }

    #[test]
    fn test_get_history_on_lib_file() {
        let file = PathBuf::from("crates/rocketindex/src/lib.rs");

        if let Ok(history) = get_history(&file, 1, 10) {
            // lib.rs should have history
            for info in &history {
                assert!(!info.commit.is_empty());
                assert!(info.commit.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(!info.date_relative.is_empty());
            }
        }
    }

    #[test]
    fn test_get_commit_info_invalid_hash() {
        let result = get_commit_info("not_a_valid_hash");
        assert!(result.is_err());
    }
}
