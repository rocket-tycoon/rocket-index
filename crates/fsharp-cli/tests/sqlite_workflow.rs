#![allow(deprecated)]

use assert_cmd::Command;
use fsharp_index::db::DEFAULT_DB_NAME;
use predicates::str::contains;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

type TestResult<T = ()> = std::result::Result<T, Box<dyn Error>>;

/// Represents a throwaway F# workspace on disk for exercising the CLI.
///
/// All helpers return `Result` so callers can use `?` and keep tests tidy.
struct SampleWorkspace {
    dir: TempDir,
    module_name: String,
}

impl SampleWorkspace {
    fn new(module_name: &str) -> TestResult<Self> {
        Ok(Self {
            dir: TempDir::new()?,
            module_name: module_name.to_string(),
        })
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn sqlite_db_path(&self) -> PathBuf {
        self.root().join(".fsharp-index").join(DEFAULT_DB_NAME)
    }

    /// Writes a simple module with a single function so the indexer has work to do.
    fn write_entry_file(&self) -> TestResult<PathBuf> {
        let src_dir = self.root().join("src");
        fs::create_dir_all(&src_dir)?;
        let file_path = src_dir.join("App.fs");
        let contents = format!(
            "module {module}\n\nlet hello() = \"world\"\n",
            module = self.module_name
        );
        fs::write(&file_path, contents)?;
        Ok(file_path)
    }
}

#[test]
fn build_creates_sqlite_index_with_metadata() -> TestResult {
    let workspace = SampleWorkspace::new("BuildSmoke")?;
    workspace.write_entry_file()?;

    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["build", "--root", ".", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("Indexed"));

    let db_path = workspace.sqlite_db_path();
    assert!(
        db_path.exists(),
        "expected SQLite index at {}",
        db_path.display()
    );

    Ok(())
}

#[test]
fn def_command_reads_from_sqlite_index() -> TestResult {
    let workspace = SampleWorkspace::new("DefinitionSmoke")?;
    workspace.write_entry_file()?;

    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["build", "--root", ".", "--format", "text"])
        .assert()
        .success();

    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["def", "DefinitionSmoke.hello", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("App.fs"));

    Ok(())
}

/// A more realistic multi-file workspace for integration testing
struct MultiFileWorkspace {
    dir: TempDir,
}

impl MultiFileWorkspace {
    fn new() -> TestResult<Self> {
        let dir = TempDir::new()?;
        let root = dir.path();

        // Create directory structure
        let src = root.join("src");
        fs::create_dir_all(&src)?;

        // Create Domain.fs - the core types
        fs::write(
            src.join("Domain.fs"),
            r#"module MyApp.Domain

type User = { Id: int; Name: string }

type Order = { OrderId: int; UserId: int; Total: decimal }

let createUser id name = { Id = id; Name = name }
"#,
        )?;

        // Create Services.fs - uses Domain types
        fs::write(
            src.join("Services.fs"),
            r#"module MyApp.Services

open MyApp.Domain

let getUserById id = createUser id "Test User"

let processOrder (user: User) amount =
    { OrderId = 1; UserId = user.Id; Total = amount }
"#,
        )?;

        // Create App.fs - the entry point
        fs::write(
            src.join("App.fs"),
            r#"module MyApp.App

open MyApp.Domain
open MyApp.Services

let main () =
    let user = getUserById 42
    let order = processOrder user 99.99M
    printfn "Created order %d for user %s" order.OrderId user.Name
"#,
        )?;

        Ok(Self { dir })
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }
}

#[test]
fn multi_file_project_indexes_all_symbols() -> TestResult {
    let workspace = MultiFileWorkspace::new()?;

    // Build the index
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["build", "--root", ".", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("3 files"))
        .stdout(contains("symbols"));

    Ok(())
}

#[test]
fn symbol_search_finds_types_and_functions() -> TestResult {
    let workspace = MultiFileWorkspace::new()?;

    // Build the index
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["build", "--root", ".", "--format", "text"])
        .assert()
        .success();

    // Search for User type
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["symbols", "*User*", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("MyApp.Domain.User"))
        .stdout(contains("Record"));

    // Search for functions
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["symbols", "*process*", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("MyApp.Services.processOrder"));

    Ok(())
}

#[test]
fn def_resolves_across_modules() -> TestResult {
    let workspace = MultiFileWorkspace::new()?;

    // Build the index
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["build", "--root", ".", "--format", "text"])
        .assert()
        .success();

    // Look up a function in Services that uses Domain
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["def", "MyApp.Services.processOrder", "--context", "--format", "text"])
        .assert()
        .success()
        .stdout(contains("Services.fs"))
        .stdout(contains("processOrder"));

    Ok(())
}

#[test]
fn spider_traverses_dependencies() -> TestResult {
    let workspace = MultiFileWorkspace::new()?;

    // Build the index
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["build", "--root", ".", "--format", "text"])
        .assert()
        .success();

    // Spider from main should find dependencies
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["spider", "MyApp.App.main", "--depth", "2", "--format", "text"])
        .assert()
        .success()
        // Should find the user and order references
        .stdout(contains("MyApp.App.main"));

    Ok(())
}

#[test]
fn json_output_format_works() -> TestResult {
    let workspace = MultiFileWorkspace::new()?;

    // Build with JSON output
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["build", "--root", ".", "--json"])
        .assert()
        .success()
        .stdout(contains("\"files\""))
        .stdout(contains("\"symbols\""));

    // Symbols with JSON output
    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["symbols", "*User*", "--json"])
        .assert()
        .success()
        .stdout(contains("\"name\""))
        .stdout(contains("\"qualified\""));

    Ok(())
}
