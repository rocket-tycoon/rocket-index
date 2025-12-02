use assert_cmd::prelude::*;
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
        .args(["build", "--root", "."])
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
        .args(["build", "--root", "."])
        .assert()
        .success();

    Command::cargo_bin("fsharp-index")?
        .current_dir(workspace.root())
        .args(["def", "DefinitionSmoke.hello"])
        .assert()
        .success()
        .stdout(contains("App.fs"));

    Ok(())
}
