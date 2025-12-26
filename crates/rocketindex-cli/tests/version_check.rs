//! Integration tests for version checking with mock HTTP server.
//!
//! These tests use wiremock to simulate GitHub API responses,
//! allowing us to test update detection without hitting the real API.
//!
//! Tests are serialized to avoid env var race conditions.

use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper to set up environment for testing
fn setup_test_env(mock_server: &MockServer) {
    // Point API to mock server
    std::env::set_var("ROCKETINDEX_GITHUB_API", mock_server.uri());
}

/// Helper to clean up test environment
fn cleanup_test_env() {
    std::env::remove_var("ROCKETINDEX_GITHUB_API");
}

#[tokio::test]
#[serial]
async fn test_fetch_detects_newer_version() {
    let mock_server = MockServer::start().await;

    // Mock GitHub releases endpoint returning a newer version
    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"tag_name": "v99.0.0"}
        ])))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);

    // Import the function we're testing
    // Note: This requires the function to be pub(crate) or pub
    let result = rocketindex_cli::version_check::fetch_latest_version();

    cleanup_test_env();

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "99.0.0");
}

#[tokio::test]
#[serial]
async fn test_fetch_handles_prerelease_version() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"tag_name": "v0.2.0-beta.1"}
        ])))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);

    let result = rocketindex_cli::version_check::fetch_latest_version();

    cleanup_test_env();

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "0.2.0-beta.1");
}

#[tokio::test]
#[serial]
async fn test_fetch_handles_empty_releases() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);

    let result = rocketindex_cli::version_check::fetch_latest_version();

    cleanup_test_env();

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No releases found"));
}

#[tokio::test]
#[serial]
async fn test_fetch_handles_api_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);

    let result = rocketindex_cli::version_check::fetch_latest_version();

    cleanup_test_env();

    assert!(result.is_err());
}

#[tokio::test]
#[serial]
async fn test_fetch_handles_rate_limit() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "message": "API rate limit exceeded"
        })))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);

    let result = rocketindex_cli::version_check::fetch_latest_version();

    cleanup_test_env();

    assert!(result.is_err());
}

#[tokio::test]
#[serial]
async fn test_fetch_handles_malformed_json() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);

    let result = rocketindex_cli::version_check::fetch_latest_version();

    cleanup_test_env();

    assert!(result.is_err());
}

#[tokio::test]
#[serial]
async fn test_check_for_update_with_newer_version() {
    let mock_server = MockServer::start().await;

    // Return a version that's definitely newer than any real version
    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"tag_name": "v99.99.99"}
        ])))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);
    rocketindex_cli::version_check::clear_cache();

    let result = rocketindex_cli::version_check::check_for_update();

    cleanup_test_env();

    assert!(result.is_some());
    let (current, latest) = result.unwrap();
    assert_eq!(latest, "99.99.99");
    assert!(!current.is_empty());
}

#[tokio::test]
#[serial]
async fn test_check_for_update_when_current() {
    let mock_server = MockServer::start().await;

    // Return the current version (get it from the module)
    let current_version = rocketindex_cli::version_check::CURRENT_VERSION;

    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([{"tag_name": format!("v{}", current_version)}])),
        )
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);
    rocketindex_cli::version_check::clear_cache();

    let result = rocketindex_cli::version_check::check_for_update();

    cleanup_test_env();

    // Should be None since we're up to date
    assert!(result.is_none());
}

#[tokio::test]
#[serial]
async fn test_check_for_update_with_older_version() {
    let mock_server = MockServer::start().await;

    // Return a version older than any possible current version
    Mock::given(method("GET"))
        .and(path("/repos/rocket-tycoon/rocket-index/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"tag_name": "v0.0.1"}
        ])))
        .mount(&mock_server)
        .await;

    setup_test_env(&mock_server);
    rocketindex_cli::version_check::clear_cache();

    let result = rocketindex_cli::version_check::check_for_update();

    cleanup_test_env();

    // Should be None since the "latest" is actually older
    assert!(result.is_none());
}
