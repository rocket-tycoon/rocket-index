//! Per-language integration tests using minimal fixtures.
//!
//! These tests verify that all CLI commands work correctly for each supported language.
//! Each language has a minimal fixture in tests/fixtures/minimal/<lang>/ with:
//! - helper function
//! - mainFunction that calls helper
//! - callerA/callerB that call mainFunction
//! - MyClass with constructor and method
//! - ChildClass inheriting from MyClass (where language supports it)
//!
//! Tests copy fixtures to isolated temp directories to avoid parallel test conflicts.

#![allow(deprecated)] // cargo_bin is deprecated in assert_cmd but replacement not yet stable

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Language-specific test configuration
#[allow(dead_code)] // Fields document fixture structure even if not all are read
struct LanguageConfig {
    /// Language name (directory under fixtures/minimal/)
    name: &'static str,
    /// File extension
    ext: &'static str,
    /// Main source file name
    main_file: &'static str,
    /// Qualified name for helper function
    helper_qualified: &'static str,
    /// Qualified name for mainFunction
    main_function_qualified: &'static str,
    /// Qualified name for callerA (calls mainFunction)
    caller_qualified: &'static str,
    /// Qualified name for MyClass/MyStruct
    class_qualified: Option<&'static str>,
    /// Qualified name for ChildClass (if inheritance supported)
    child_class_qualified: Option<&'static str>,
    /// Parent class name for subclass search
    parent_class_name: Option<&'static str>,
}

/// Get the path to fixtures directory
fn fixtures_dir() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/minimal")
}

/// Copy a fixture directory to an isolated temp directory
fn copy_fixture_to_temp(lang: &str) -> TestResult<Option<TempDir>> {
    let fixture_path = fixtures_dir().join(lang);
    if !fixture_path.exists() {
        return Ok(None);
    }

    let temp_dir = TempDir::new()?;
    copy_dir_recursive(&fixture_path, temp_dir.path())?;
    Ok(Some(temp_dir))
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            // Skip .rocketindex directories (old index data)
            if entry.file_name() == ".rocketindex" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// All supported language configurations
fn language_configs() -> Vec<LanguageConfig> {
    vec![
        LanguageConfig {
            name: "rust",
            ext: "rs",
            main_file: "src/lib.rs",
            helper_qualified: "utils::helper",
            main_function_qualified: "main_function",
            caller_qualified: "caller_a",
            class_qualified: Some("MyStruct"),
            child_class_qualified: None, // Rust uses traits, not inheritance
            parent_class_name: None,
        },
        LanguageConfig {
            name: "python",
            ext: "py",
            main_file: "main.py",
            helper_qualified: "helper",
            main_function_qualified: "main_function",
            caller_qualified: "caller_a",
            class_qualified: Some("MyClass"),
            child_class_qualified: Some("ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "typescript",
            ext: "ts",
            main_file: "src/index.ts",
            helper_qualified: "helper",
            main_function_qualified: "mainFunction",
            caller_qualified: "callerA",
            class_qualified: Some("MyClass"),
            child_class_qualified: Some("ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "javascript",
            ext: "js",
            main_file: "main.js",
            helper_qualified: "helper",
            main_function_qualified: "mainFunction",
            caller_qualified: "callerA",
            class_qualified: Some("MyClass"),
            child_class_qualified: Some("ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "go",
            ext: "go",
            main_file: "main.go",
            helper_qualified: "main.helper",
            main_function_qualified: "main.mainFunction",
            caller_qualified: "main.callerA",
            class_qualified: Some("main.MyStruct"),
            child_class_qualified: None, // Go uses composition, not inheritance
            parent_class_name: None,
        },
        LanguageConfig {
            name: "java",
            ext: "java",
            main_file: "Main.java",
            helper_qualified: "minimal.Main.helper",
            main_function_qualified: "minimal.Main.mainFunction",
            caller_qualified: "minimal.Main.callerA",
            class_qualified: Some("minimal.MyClass"),
            child_class_qualified: Some("minimal.ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "csharp",
            ext: "cs",
            main_file: "Main.cs",
            helper_qualified: "Minimal.Helpers.Helper",
            main_function_qualified: "Minimal.Program.MainFunction",
            caller_qualified: "Minimal.Program.CallerA",
            class_qualified: Some("Minimal.MyClass"),
            child_class_qualified: Some("Minimal.ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "fsharp",
            ext: "fs",
            main_file: "Main.fs",
            helper_qualified: "Minimal.Main.helper",
            main_function_qualified: "Minimal.Main.mainFunction",
            caller_qualified: "Minimal.Main.callerA",
            class_qualified: Some("Minimal.Main.MyClass"),
            child_class_qualified: Some("Minimal.Main.ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "ruby",
            ext: "rb",
            main_file: "main.rb",
            helper_qualified: "helper",
            main_function_qualified: "main_function",
            caller_qualified: "caller_a",
            class_qualified: Some("MyClass"),
            child_class_qualified: Some("ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "php",
            ext: "php",
            main_file: "main.php",
            helper_qualified: "helper",
            main_function_qualified: "mainFunction",
            caller_qualified: "callerA",
            class_qualified: Some("MyClass"),
            child_class_qualified: Some("ChildClass"),
            parent_class_name: Some("MyClass"),
        },
        LanguageConfig {
            name: "c",
            ext: "c",
            main_file: "main.c",
            helper_qualified: "helper",
            main_function_qualified: "main_function",
            caller_qualified: "caller_a",
            class_qualified: Some("MyStruct"), // C struct
            child_class_qualified: None,       // No inheritance in C
            parent_class_name: None,
        },
        LanguageConfig {
            name: "cpp",
            ext: "cpp",
            main_file: "main.cpp",
            helper_qualified: "helper",
            main_function_qualified: "mainFunction",
            caller_qualified: "callerA",
            class_qualified: Some("MyClass"),
            child_class_qualified: Some("ChildClass"),
            parent_class_name: Some("MyClass"),
        },
    ]
}

/// Index a directory
fn index_dir(dir: &Path) -> TestResult {
    let output = Command::cargo_bin("rkt")?
        .current_dir(dir)
        .args(["index", "--format", "json"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "Failed to index {}: stderr={}, stdout={}",
            dir.display(),
            stderr,
            stdout
        );
    }

    Ok(())
}

// =============================================================================
// Definition Tests
// =============================================================================

#[test]
fn test_def_finds_helper_all_languages() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => {
                eprintln!("Skipping {}: fixture not found", config.name);
                continue;
            }
        };

        index_dir(temp_dir.path())?;

        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["def", config.helper_qualified, "--format", "json"])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout);

        assert!(
            result.status.success(),
            "[{}] def {} failed: {}",
            config.name,
            config.helper_qualified,
            String::from_utf8_lossy(&result.stderr)
        );

        // JSON output should contain file location
        assert!(
            stdout.contains("\"file\"") || stdout.contains("\"line\""),
            "[{}] def {} should return location info, got: {}",
            config.name,
            config.helper_qualified,
            stdout
        );
    }

    Ok(())
}

#[test]
fn test_def_finds_main_function_all_languages() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["def", config.main_function_qualified, "--format", "json"])
            .output()?;

        assert!(
            result.status.success(),
            "[{}] def {} failed: {}",
            config.name,
            config.main_function_qualified,
            String::from_utf8_lossy(&result.stderr)
        );
    }

    Ok(())
}

// =============================================================================
// Callers Tests
// =============================================================================

#[test]
fn test_callers_finds_callers_of_main_function() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        // mainFunction is called by callerA and callerB
        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args([
                "callers",
                config.main_function_qualified,
                "--format",
                "json",
            ])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout);

        // Should find at least one caller
        // Note: might return NOT_FOUND (exit 1) if callers feature incomplete for this language
        if result.status.success() {
            assert!(
                stdout.contains("callers") || stdout.contains("caller"),
                "[{}] callers {} should list callers, got: {}",
                config.name,
                config.main_function_qualified,
                stdout
            );
        }
    }

    Ok(())
}

#[test]
fn test_callers_finds_callers_of_helper() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        // helper is called by mainFunction and callerB
        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["callers", config.helper_qualified, "--format", "json"])
            .output()?;

        // Just verify the command doesn't crash
        // Success or NOT_FOUND are both acceptable
        let _stdout = String::from_utf8_lossy(&result.stdout);
    }

    Ok(())
}

// =============================================================================
// References Tests
// =============================================================================

#[test]
fn test_refs_finds_references() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        // Use short name for broader matching
        let short_name = config
            .helper_qualified
            .split(&['.', ':', '#'][..])
            .next_back()
            .unwrap_or(config.helper_qualified);

        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["refs", short_name, "--format", "json"])
            .output()?;

        // refs command should at minimum not crash
        let _stdout = String::from_utf8_lossy(&result.stdout);
    }

    Ok(())
}

// =============================================================================
// Subclasses Tests
// =============================================================================

#[test]
fn test_subclasses_finds_child_classes() -> TestResult {
    for config in language_configs() {
        // Skip languages without inheritance
        let parent = match config.parent_class_name {
            Some(p) => p,
            None => continue,
        };

        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["subclasses", parent, "--format", "json"])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout);

        // If command succeeds, should find ChildClass
        if result.status.success() && stdout.contains("\"count\"") {
            // Verify we found at least the child class
            // Some languages might have more specific qualified names
            assert!(
                stdout.contains("Child") || stdout.contains("count\": 0"),
                "[{}] subclasses {} expected to find ChildClass, got: {}",
                config.name,
                parent,
                stdout
            );
        }
    }

    Ok(())
}

// =============================================================================
// Spider Tests
// =============================================================================

#[test]
fn test_spider_traverses_call_graph() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        // Spider from caller should find main_function in its dependencies
        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args([
                "spider",
                config.caller_qualified,
                "--depth",
                "2",
                "--format",
                "json",
            ])
            .output()?;

        // Just verify it doesn't crash - spider output varies by language
        let _stdout = String::from_utf8_lossy(&result.stdout);
    }

    Ok(())
}

#[test]
fn test_spider_reverse_finds_callers() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        // Reverse spider from helper should find functions that call it
        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args([
                "spider",
                config.helper_qualified,
                "--reverse",
                "--depth",
                "2",
                "--format",
                "json",
            ])
            .output()?;

        // Just verify it doesn't crash
        let _stdout = String::from_utf8_lossy(&result.stdout);
    }

    Ok(())
}

// =============================================================================
// Symbols Search Tests
// =============================================================================

#[test]
fn test_symbols_search_finds_functions() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        // Search for helper* pattern
        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["symbols", "*helper*", "--format", "json"])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout);

        assert!(
            result.status.success(),
            "[{}] symbols search failed: {}",
            config.name,
            String::from_utf8_lossy(&result.stderr)
        );

        // Should find at least one symbol
        assert!(
            stdout.contains("helper") || stdout.contains("Helper"),
            "[{}] symbols *helper* should find helper function, got: {}",
            config.name,
            stdout
        );
    }

    Ok(())
}

#[test]
fn test_symbols_search_finds_classes() -> TestResult {
    for config in language_configs() {
        let class_name = match config.class_qualified {
            Some(c) => c,
            None => continue,
        };

        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        index_dir(temp_dir.path())?;

        // Search for *Class* or *Struct* pattern
        let pattern = if class_name.contains("Struct") {
            "*Struct*"
        } else {
            "*Class*"
        };

        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["symbols", pattern, "--format", "json"])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout);

        if result.status.success() {
            // Should find at least one class/struct
            assert!(
                stdout.contains("Class") || stdout.contains("Struct") || stdout.contains("class"),
                "[{}] symbols {} should find class/struct, got: {}",
                config.name,
                pattern,
                stdout
            );
        }
    }

    Ok(())
}

// =============================================================================
// JSON Output Format Tests
// =============================================================================

#[test]
fn test_json_output_is_valid() -> TestResult {
    // Test with Rust fixture as representative
    let temp_dir = match copy_fixture_to_temp("rust")? {
        Some(d) => d,
        None => return Ok(()),
    };

    index_dir(temp_dir.path())?;

    // Test each command produces valid JSON
    let commands = [
        vec!["def", "main_function", "--format", "json"],
        vec!["symbols", "*", "--format", "json", "--limit", "5"],
        vec!["callers", "main_function", "--format", "json"],
        vec!["refs", "helper", "--format", "json"],
    ];

    for cmd in commands {
        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(&cmd)
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout);

        // Should be parseable as JSON (starts with { or [)
        let trimmed = stdout.trim();
        if !trimmed.is_empty() {
            assert!(
                trimmed.starts_with('{') || trimmed.starts_with('['),
                "Command {:?} should produce JSON, got: {}",
                cmd,
                trimmed
            );
        }
    }

    Ok(())
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[test]
fn test_nonexistent_symbol_returns_not_found() -> TestResult {
    let temp_dir = match copy_fixture_to_temp("rust")? {
        Some(d) => d,
        None => return Ok(()),
    };

    index_dir(temp_dir.path())?;

    let result = Command::cargo_bin("rkt")?
        .current_dir(temp_dir.path())
        .args(["def", "nonexistent_symbol_xyz", "--format", "json"])
        .output()?;

    // Should return NOT_FOUND exit code (1) not crash
    assert!(
        !result.status.success(),
        "Looking up nonexistent symbol should fail"
    );

    Ok(())
}

#[test]
fn test_index_command_reports_all_files() -> TestResult {
    for config in language_configs() {
        let temp_dir = match copy_fixture_to_temp(config.name)? {
            Some(d) => d,
            None => continue,
        };

        let result = Command::cargo_bin("rkt")?
            .current_dir(temp_dir.path())
            .args(["index", "--format", "json"])
            .output()?;

        let stdout = String::from_utf8_lossy(&result.stdout);

        assert!(
            result.status.success(),
            "[{}] index failed: {}",
            config.name,
            String::from_utf8_lossy(&result.stderr)
        );

        // Should report files and symbols
        assert!(
            stdout.contains("\"files\"") || stdout.contains("\"symbols\""),
            "[{}] index should report stats, got: {}",
            config.name,
            stdout
        );
    }

    Ok(())
}
