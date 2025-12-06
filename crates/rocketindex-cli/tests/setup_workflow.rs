#![allow(deprecated)] // cargo_bin is deprecated but still works

use assert_cmd::Command;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

type TestResult<T = ()> = std::result::Result<T, Box<dyn Error>>;

struct SetupWorkspace {
    dir: TempDir,
}

impl SetupWorkspace {
    fn new() -> TestResult<Self> {
        Ok(Self {
            dir: TempDir::new()?,
        })
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn path(&self, path: &str) -> PathBuf {
        self.root().join(path)
    }

    fn assert_exists(&self, path: &str) {
        let p = self.path(path);
        assert!(p.exists(), "File did not exist: {}", p.display());
    }

    fn assert_not_exists(&self, path: &str) {
        let p = self.path(path);
        assert!(!p.exists(), "File SHOULD NOT exist: {}", p.display());
    }

    fn read_file(&self, path: &str) -> TestResult<String> {
        let p = self.path(path);
        Ok(fs::read_to_string(p)?)
    }
}

#[test]
fn setup_claude_creates_correct_files() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create a dummy CLAUDE.md to verifying updating
    fs::write(
        workspace.path("CLAUDE.md"),
        "# My Project\n\nSome existing content.\n",
    )?;

    // Run setup claude
    // Note: We use --quiet to avoid spinner output issues in tests
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "claude", "--quiet"])
        .assert()
        .success();

    // 1. Verify AGENTS.md creation in .rocketindex/
    workspace.assert_exists(".rocketindex/AGENTS.md");
    let agents_content = workspace.read_file(".rocketindex/AGENTS.md")?;
    assert!(
        agents_content.contains("RocketIndex") && agents_content.contains("rkt callers"),
        "AGENTS.md content incorrect"
    );

    // 2. Verify /ri slash command is NOT created (redundant)
    workspace.assert_not_exists(".claude/commands/ri.md");

    // 3. Verify CLAUDE.md update
    let claude_md = workspace.read_file("CLAUDE.md")?;
    assert!(
        claude_md.contains(".rocketindex/AGENTS.md"),
        "CLAUDE.md should reference .rocketindex/AGENTS.md"
    );

    // 4. Verify Copilot instructions
    workspace.assert_exists(".github/copilot-instructions.md");

    Ok(())
}

#[test]
fn setup_cursor_creates_rules() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run setup cursor
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "cursor", "--quiet"])
        .assert()
        .success();

    // Verify .cursor/rules creation
    workspace.assert_exists(".cursor/rules");
    let rules = workspace.read_file(".cursor/rules")?;
    assert!(
        rules.contains("RocketIndex Code Navigation"),
        "Cursor rules content incorrect"
    );
    assert!(rules.contains("rkt index"), "Cursor rules incorrect");

    Ok(())
}
