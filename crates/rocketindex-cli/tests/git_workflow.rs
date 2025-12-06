#![allow(deprecated)] // cargo_bin is deprecated but still works

use assert_cmd::Command;
use predicates::str::contains;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use tempfile::TempDir;

type TestResult<T = ()> = std::result::Result<T, Box<dyn Error>>;

struct GitWorkspace {
    dir: TempDir,
}

impl GitWorkspace {
    fn new() -> TestResult<Self> {
        let dir = TempDir::new()?;
        let root = dir.path();

        // Initialize git repo
        StdCommand::new("git")
            .arg("init")
            .current_dir(root)
            .output()?;

        // Configure git user for commits
        StdCommand::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(root)
            .output()?;
        StdCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(root)
            .output()?;

        Ok(Self { dir })
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn commit_file(&self, path: &str, content: &str, message: &str) -> TestResult {
        let file_path = self.root().join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, content)?;

        StdCommand::new("git")
            .args(["add", path])
            .current_dir(self.root())
            .output()?;

        StdCommand::new("git")
            .args(["commit", "-m", message])
            .current_dir(self.root())
            .output()?;

        Ok(())
    }
}

#[test]
fn blame_command_shows_commit_info() -> TestResult {
    let workspace = GitWorkspace::new()?;

    workspace.commit_file(
        "src/App.fs",
        "module App\n\nlet hello() = \"world\"\n",
        "Initial commit",
    )?;

    // Build index
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["build", "--root", "."])
        .assert()
        .success();

    // Test blame
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["blame", "App.hello", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("Initial commit"))
        .stdout(contains("Test User"));

    Ok(())
}

#[test]
fn history_command_shows_log() -> TestResult {
    let workspace = GitWorkspace::new()?;

    workspace.commit_file(
        "src/App.fs",
        "module App\n\nlet hello() = \"world\"\n",
        "Initial commit",
    )?;

    // Modify file
    workspace.commit_file(
        "src/App.fs",
        "module App\n\nlet hello() = \"modified\"\n",
        "Update hello",
    )?;

    // Build index
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["build", "--root", "."])
        .assert()
        .success();

    // Test history
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["history", "App.hello", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("Initial commit"))
        .stdout(contains("Update hello"));

    Ok(())
}

#[test]
fn def_git_flag_shows_provenance() -> TestResult {
    let workspace = GitWorkspace::new()?;

    workspace.commit_file(
        "src/App.fs",
        "module App\n\nlet hello() = \"world\"\n",
        "Feature commit",
    )?;

    // Build index
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["build", "--root", "."])
        .assert()
        .success();

    // Test def --git
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["def", "App.hello", "--git", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("Feature commit"))
        .stdout(contains("Test User"));

    Ok(())
}
