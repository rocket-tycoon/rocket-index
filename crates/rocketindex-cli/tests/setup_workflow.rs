#![allow(deprecated)] // cargo_bin is deprecated in assert_cmd but replacement not yet stable

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

    workspace.assert_exists(".rocketindex/index.db");

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

    workspace.assert_exists(".rocketindex/index.db");

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

    workspace.assert_exists(".rocketindex/index.db");

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

    workspace.assert_exists(".rocketindex/index.db");

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

    workspace.assert_exists(".rocketindex/index.db");

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

    workspace.assert_exists(".rocketindex/index.db");

    let first_content = workspace.read_file(".github/copilot-instructions.md")?;

    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "copilot", "--quiet"])
        .assert()
        .success();

    workspace.assert_exists(".rocketindex/index.db");

    let second_content = workspace.read_file(".github/copilot-instructions.md")?;

    // Content should be identical - no duplicate sections
    assert_eq!(
        first_content, second_content,
        "Running setup copilot twice should produce identical content"
    );

    Ok(())
}

// ============================================================================
// rkt start tests
// ============================================================================

#[test]
fn start_rejects_invalid_agent() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run start with invalid agent
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["start", "invalid-agent", "--quiet"])
        .assert()
        .code(2); // ERROR exit code

    Ok(())
}

#[test]
fn start_rejects_invalid_agent_json_output() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run start with invalid agent, JSON format
    let output = Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["start", "invalid-agent", "--format", "json"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"error\"") && stdout.contains("Unknown agent"),
        "Should output JSON error for invalid agent"
    );

    Ok(())
}

#[test]
fn start_accepts_valid_agent_names() -> TestResult {
    // Test that valid agent names are accepted (command starts but we can't
    // test the blocking watch, so we just verify setup runs)
    let valid_agents = [
        "claude",
        "claude-code",
        "cursor",
        "copilot",
        "github-copilot",
        "zed",
        "gemini",
    ];

    for agent in valid_agents {
        let workspace = SetupWorkspace::new()?;

        // Create an index so it doesn't run full setup wizard
        // Then start will try to run watch, which will block
        // We use timeout to kill it after setup verification
        fs::create_dir_all(workspace.path(".rocketindex"))?;

        // We can't easily test the full flow since watch blocks,
        // but we can at least verify the agent name is accepted
        // by checking it doesn't immediately fail with "Unknown agent"
        let output = Command::cargo_bin("rkt")?
            .current_dir(workspace.root())
            .args(["start", agent, "--format", "json"])
            .timeout(std::time::Duration::from_millis(500))
            .output();

        // Either it times out (watch started) or fails for another reason
        // but NOT because of invalid agent name
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                !stdout.contains("Unknown agent"),
                "Agent '{}' should be recognized as valid",
                agent
            );
        }
        // Timeout is expected and acceptable - means watch tried to start
    }

    Ok(())
}

// ============================================================================
// Zed setup tests
// ============================================================================

#[test]
fn setup_zed_creates_correct_files() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run setup zed
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "zed", "--quiet"])
        .assert()
        .success();

    // Verify index was created
    workspace.assert_exists(".rocketindex/index.db");

    // Verify AGENTS.md creation in .rocketindex/
    workspace.assert_exists(".rocketindex/AGENTS.md");
    let agents_content = workspace.read_file(".rocketindex/AGENTS.md")?;
    assert!(
        agents_content.contains("RocketIndex") && agents_content.contains("rkt callers"),
        "AGENTS.md content incorrect"
    );

    // Verify .rules file creation (Zed's primary rules file)
    workspace.assert_exists(".rules");
    let rules = workspace.read_file(".rules")?;
    assert!(
        rules.contains("RocketIndex"),
        ".rules should contain RocketIndex instructions"
    );
    assert!(rules.contains("rkt"), ".rules should contain rkt commands");

    Ok(())
}

#[test]
fn setup_zed_updates_existing_rules_file() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create existing .rules file
    fs::write(
        workspace.path(".rules"),
        "# My Project Rules\n\nExisting rules for my project.\n",
    )?;

    // Run setup zed
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "zed", "--quiet"])
        .assert()
        .success();

    workspace.assert_exists(".rocketindex/index.db");

    // Verify content was appended, not replaced
    let rules = workspace.read_file(".rules")?;
    assert!(
        rules.contains("Existing rules for my project"),
        "Original content should be preserved"
    );
    assert!(
        rules.contains("RocketIndex"),
        "RocketIndex section should be added"
    );

    Ok(())
}

#[test]
fn setup_zed_idempotent() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run setup zed twice
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "zed", "--quiet"])
        .assert()
        .success();

    workspace.assert_exists(".rocketindex/index.db");

    let first_content = workspace.read_file(".rules")?;

    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "zed", "--quiet"])
        .assert()
        .success();

    let second_content = workspace.read_file(".rules")?;

    // Content should be identical - no duplicate sections
    assert_eq!(
        first_content, second_content,
        "Running setup zed twice should produce identical content"
    );

    Ok(())
}

#[test]
fn setup_zed_also_updates_claude_md_if_exists() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create existing CLAUDE.md (Zed also reads this file)
    fs::write(
        workspace.path("CLAUDE.md"),
        "# My Project\n\nProject documentation.\n",
    )?;

    // Run setup zed
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "zed", "--quiet"])
        .assert()
        .success();

    workspace.assert_exists(".rocketindex/index.db");

    // Verify CLAUDE.md was updated with RocketIndex reference
    let claude_md = workspace.read_file("CLAUDE.md")?;
    assert!(
        claude_md.contains(".rocketindex/AGENTS.md"),
        "CLAUDE.md should reference .rocketindex/AGENTS.md"
    );
    assert!(
        claude_md.contains("Project documentation"),
        "Original content should be preserved"
    );

    // Verify .rules also exists
    workspace.assert_exists(".rules");

    Ok(())
}

#[test]
fn start_zed_is_recognized() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create an index so it doesn't run full setup wizard
    fs::create_dir_all(workspace.path(".rocketindex"))?;

    // Verify 'zed' is accepted as a valid agent
    let output = Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["start", "zed", "--format", "json"])
        .timeout(std::time::Duration::from_millis(500))
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.contains("Unknown agent"),
            "Agent 'zed' should be recognized as valid"
        );
    }
    // Timeout is expected and acceptable - means watch tried to start

    Ok(())
}

// ============================================================================
// Gemini CLI setup tests
// ============================================================================

#[test]
fn setup_gemini_creates_correct_files() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run setup gemini
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "gemini", "--quiet"])
        .assert()
        .success();

    // Verify index was created
    workspace.assert_exists(".rocketindex/index.db");

    // Verify AGENTS.md creation in .rocketindex/
    workspace.assert_exists(".rocketindex/AGENTS.md");
    let agents_content = workspace.read_file(".rocketindex/AGENTS.md")?;
    assert!(
        agents_content.contains("RocketIndex") && agents_content.contains("rkt callers"),
        "AGENTS.md content incorrect"
    );

    // Verify GEMINI.md file creation (Gemini CLI's default context file)
    workspace.assert_exists("GEMINI.md");
    let gemini_md = workspace.read_file("GEMINI.md")?;
    assert!(
        gemini_md.contains("RocketIndex"),
        "GEMINI.md should contain RocketIndex instructions"
    );
    assert!(
        gemini_md.contains("rkt"),
        "GEMINI.md should contain rkt commands"
    );

    Ok(())
}

#[test]
fn setup_gemini_updates_existing_gemini_md() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create existing GEMINI.md file
    fs::write(
        workspace.path("GEMINI.md"),
        "# My Project\n\nExisting project instructions.\n",
    )?;

    // Run setup gemini
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "gemini", "--quiet"])
        .assert()
        .success();

    workspace.assert_exists(".rocketindex/index.db");

    // Verify content was appended, not replaced
    let gemini_md = workspace.read_file("GEMINI.md")?;
    assert!(
        gemini_md.contains("Existing project instructions"),
        "Original content should be preserved"
    );
    assert!(
        gemini_md.contains("RocketIndex"),
        "RocketIndex section should be added"
    );

    Ok(())
}

#[test]
fn setup_gemini_idempotent() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Run setup gemini twice
    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "gemini", "--quiet"])
        .assert()
        .success();

    workspace.assert_exists(".rocketindex/index.db");

    let first_content = workspace.read_file("GEMINI.md")?;

    Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["setup", "gemini", "--quiet"])
        .assert()
        .success();

    let second_content = workspace.read_file("GEMINI.md")?;

    // Content should be identical - no duplicate sections
    assert_eq!(
        first_content, second_content,
        "Running setup gemini twice should produce identical content"
    );

    Ok(())
}

#[test]
fn start_gemini_is_recognized() -> TestResult {
    let workspace = SetupWorkspace::new()?;

    // Create an index so it doesn't run full setup wizard
    fs::create_dir_all(workspace.path(".rocketindex"))?;

    // Verify 'gemini' is accepted as a valid agent
    let output = Command::cargo_bin("rkt")?
        .current_dir(workspace.root())
        .args(["start", "gemini", "--format", "json"])
        .timeout(std::time::Duration::from_millis(500))
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.contains("Unknown agent"),
            "Agent 'gemini' should be recognized as valid"
        );
    }
    // Timeout is expected and acceptable - means watch tried to start

    Ok(())
}
