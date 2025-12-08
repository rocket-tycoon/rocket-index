use super::*;
use crate::mcp::tools::definition::{find_definition, FindDefinitionInput};
use crate::mcp::tools::structure::{describe_project, DescribeProjectInput};
use rocketindex::{Location, SqliteIndex, Symbol, SymbolKind, Visibility};
use std::sync::Arc;
use tempfile::TempDir;

async fn setup_project() -> (TempDir, Arc<ProjectManager>) {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Create a file
    let file_path = src.join("Models.py");
    std::fs::write(&file_path, "class User:\n pass").unwrap();

    // Create DB
    let db_path = root.join(".rocketindex").join("index.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let index = SqliteIndex::create(&db_path).unwrap();

    // Insert symbol
    let sym = Symbol::new(
        "User".to_string(),
        "Models.User".to_string(),
        SymbolKind::Class,
        Location::new(file_path.clone(), 1, 0),
        Visibility::Public,
        "python".to_string(),
    )
    .with_doc(Some("User class doc".to_string()));
    index.insert_symbol(&sym).unwrap();
    drop(index); // Release lock

    // Setup Manager
    // We construct a ProjectManager, which reads global config (safe as we don't save back)
    // Then we manually "register" the project in memory
    let manager = ProjectManager::new().await.unwrap();
    manager
        .register_in_memory(root.to_path_buf())
        .await
        .unwrap();

    // Wrap in Arc as expected by tools
    (dir, Arc::new(manager))
}

#[tokio::test]
async fn test_fuzzy_fallback_success() {
    let (_dir, manager) = setup_project().await;

    // Search for typo "Usr"
    let input = FindDefinitionInput {
        symbol: "Usr".to_string(),
        file: None,
        project_root: None,
        include_context: false,
    };

    let result = find_definition(manager, input).await;

    // Result should be success with JSON
    let json = serde_json::to_string(&result).unwrap();

    // Should verify success (isError: false or missing)
    assert!(!json.contains("\"isError\":true"));

    // Should match "User" fuzzily
    assert!(json.contains("User"));
    assert!(json.contains("fuzzy"));
    assert!(json.contains("confidence"));
}

#[tokio::test]
async fn test_describe_project() {
    let (dir, manager) = setup_project().await;

    let input = DescribeProjectInput {
        path: dir.path().to_str().unwrap().to_string(),
    };

    let result = describe_project(manager, input).await;
    let json = serde_json::to_string(&result).unwrap();

    println!("Describe result:\n{}", json);

    assert!(!json.contains("\"isError\":true"));
    assert!(json.contains("# Project Map"));
    // Checking output format
    // Should verify it lists Models.py
    // Note: sqlite list_files returns absolute paths usually, but code tries to strip prefix
    assert!(json.contains("Models.py"));
    // Should list User symbol
    assert!(json.contains("User"));
}

#[tokio::test]
async fn test_staleness_warning() {
    let (dir, manager) = setup_project().await;
    let root = dir.path();
    let file_path = root.join("src").join("Models.py");

    // Sleep briefly to ensure filesystem time difference (rendering filesystem granularity)
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Touch the file -> make it newer than index.db
    std::fs::write(&file_path, "class User:\n pass # updated").unwrap();

    let input = FindDefinitionInput {
        symbol: "User".to_string(),
        file: None,
        project_root: None,
        include_context: false,
    };

    let result = find_definition(manager, input).await;
    let json = serde_json::to_string(&result).unwrap();

    // specific staleness check
    assert!(json.contains("Warning: Index may be stale"));
}

#[tokio::test]
async fn test_jit_describe_project() {
    // Setup without manual register
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let db_path = root.join(".rocketindex").join("index.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let index = SqliteIndex::create(&db_path).unwrap();
    // Drop index to release lock
    drop(index);

    let manager = ProjectManager::new().await.unwrap();
    // Do NOT register manually

    let input = DescribeProjectInput {
        path: root.to_str().unwrap().to_string(),
    };

    let result = describe_project(Arc::new(manager), input).await;
    let json = serde_json::to_string(&result).unwrap();

    // Should succeed because index exists, so ensure_registered succeeds
    assert!(!json.contains("\"isError\":true"));
    assert!(json.contains("# Project Map"));
}

#[tokio::test]
async fn test_find_definition_hint() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let file_path = root.join("foo.rs"); // Unregistered file
    let manager = Arc::new(ProjectManager::new().await.unwrap());

    let input = FindDefinitionInput {
        symbol: "foo".to_string(),
        file: Some(file_path.to_str().unwrap().to_string()),
        project_root: None,
        include_context: false,
    };

    let result = find_definition(manager, input).await;
    let json = serde_json::to_string(&result).unwrap();

    // Should error with hint
    assert!(json.contains("\"isError\":true"));
    assert!(json.contains("Use `describe_project`"));
}
