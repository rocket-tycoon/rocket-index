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

    // 4. Verify Copilot instructions NOT created (only updates if exists)
    workspace.assert_not_exists(".github/copilot-instructions.md");

    Ok(())
}

#[test]
fn setup_claude_updates_existing_copilot_instructions() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create existing files
    fs::write(workspace.path("CLAUDE.md"), "# My Project\n\nContent.\n")?;
    fs::create_dir_all(workspace.path(".github"))?;
    fs::write(
        workspace.path(".github/copilot-instructions.md"),
        "# Copilot Instructions\n\nExisting content.\n",
    )?;

    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "claude", "--quiet"])
        .assert()
        .success();

    // Verify copilot-instructions.md was updated with note
    let copilot = workspace.read_file(".github/copilot-instructions.md")?;
    assert!(
        copilot.contains(".rocketindex/AGENTS.md"),
        "copilot-instructions.md should reference AGENTS.md"
    );
    assert!(
        copilot.contains("Existing content"),
        "Original content should be preserved"
    );

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

#[test]
fn setup_copilot_creates_correct_files() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run setup copilot
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "copilot", "--quiet"])
        .assert()
        .success();

    // Verify .github/copilot-instructions.md creation
    workspace.assert_exists(".github/copilot-instructions.md");
    let copilot = workspace.read_file(".github/copilot-instructions.md")?;
    assert!(
        copilot.contains("RocketIndex Code Navigation"),
        "Copilot instructions should contain RocketIndex section"
    );
    assert!(
        copilot.contains("rkt def"),
        "Copilot instructions should contain rkt commands"
    );

    Ok(())
}

#[test]
fn setup_copilot_updates_existing_file() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create existing copilot-instructions.md
    fs::create_dir_all(workspace.path(".github"))?;
    fs::write(
        workspace.path(".github/copilot-instructions.md"),
        "# Copilot Instructions\n\nExisting content about my project.\n",
    )?;

    // Run setup copilot
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "copilot", "--quiet"])
        .assert()
        .success();

    // Verify content was appended, not replaced
    let copilot = workspace.read_file(".github/copilot-instructions.md")?;
    assert!(
        copilot.contains("Existing content about my project"),
        "Original content should be preserved"
    );
    assert!(
        copilot.contains("RocketIndex Code Navigation"),
        "RocketIndex section should be added"
    );

    Ok(())
}

#[test]
fn setup_copilot_idempotent() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run setup copilot twice
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "copilot", "--quiet"])
        .assert()
        .success();

    let first_content = workspace.read_file(".github/copilot-instructions.md")?;

    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "copilot", "--quiet"])
        .assert()
        .success();

    let second_content = workspace.read_file(".github/copilot-instructions.md")?;

    // Content should be identical - no duplicate sections
    assert_eq!(
        first_content, second_content,
        "Running setup copilot twice should produce identical content"
    );

    Ok(())
}
