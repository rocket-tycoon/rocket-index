//! Version check and self-update module.
//!
//! Queries GitHub releases API and caches results for 24 hours.
//! Can auto-update for non-Homebrew installations.

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Cache TTL: 24 hours
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// GitHub API URL for releases (includes prereleases)
const GITHUB_API_URL: &str =
    "https://api.github.com/repos/rocket-tycoon/rocket-index/releases?per_page=1";

/// Current version from Cargo.toml
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cached version check result
#[derive(Debug, Serialize, Deserialize)]
struct VersionCache {
    latest_version: String,
    checked_at: u64, // Unix timestamp
}

/// GitHub release response (minimal fields we need)
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

/// Returns the path to the version cache file
fn cache_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rocketindex")
        .join("version_cache.json")
}

/// Load cached version if still valid
fn load_cache() -> Option<String> {
    let path = cache_path();
    let contents = std::fs::read_to_string(&path).ok()?;
    let cache: VersionCache = serde_json::from_str(&contents).ok()?;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();

    // Check if cache is still valid
    if now.saturating_sub(cache.checked_at) < CACHE_TTL.as_secs() {
        Some(cache.latest_version)
    } else {
        None
    }
}

/// Save version to cache
fn save_cache(version: &str) -> Result<()> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let cache = VersionCache {
        latest_version: version.to_string(),
        checked_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs(),
    };

    let contents = serde_json::to_string_pretty(&cache)?;
    std::fs::write(path, contents)?;
    Ok(())
}

/// Fetch latest version from GitHub API
fn fetch_latest_version() -> Result<String> {
    let releases: Vec<GitHubRelease> = ureq::get(GITHUB_API_URL)
        .set("User-Agent", "rocketindex-cli")
        .set("Accept", "application/vnd.github.v3+json")
        .call()?
        .into_json()?;

    let release = releases
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No releases found"))?;

    // Strip 'v' prefix if present
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    Ok(version.to_string())
}

/// Parse version string, handling pre-release versions
fn parse_version(version_str: &str) -> Option<Version> {
    Version::parse(version_str).ok()
}

/// Check if an update is available.
///
/// Returns `Some((current, latest))` if a newer version exists,
/// `None` if current version is up-to-date or check fails.
///
/// Results are cached for 24 hours to avoid hitting the API repeatedly.
pub fn check_for_update() -> Option<(String, String)> {
    // Try cache first
    let latest = match load_cache() {
        Some(v) => v,
        None => {
            // Fetch from GitHub (ignore errors silently - don't block on network issues)
            let fetched = fetch_latest_version().ok()?;
            let _ = save_cache(&fetched);
            fetched
        }
    };

    // Compare versions
    let current = parse_version(CURRENT_VERSION)?;
    let latest_parsed = parse_version(&latest)?;

    if latest_parsed > current {
        Some((CURRENT_VERSION.to_string(), latest))
    } else {
        None
    }
}

/// Print update notification to stderr if available.
///
/// Uses stderr because stdout is reserved for MCP protocol messages.
pub fn print_update_notification() {
    if let Some((current, latest)) = check_for_update() {
        eprintln!(
            "\x1b[33m⬆ RocketIndex v{} available (current: v{})\x1b[0m",
            latest, current
        );
        if is_homebrew_install() {
            eprintln!("\x1b[33m  Update: brew upgrade rocket-tycoon/tap/rocket-index\x1b[0m");
        } else {
            eprintln!("\x1b[33m  Update: rkt update\x1b[0m");
        }
        eprintln!();
    }
}

/// Check if running from a Homebrew installation
fn is_homebrew_install() -> bool {
    std::env::current_exe()
        .map(|p| {
            let path = p.to_string_lossy();
            path.contains("homebrew") || path.contains("Cellar")
        })
        .unwrap_or(false)
}

/// Check if running from a Scoop installation (Windows)
fn is_scoop_install() -> bool {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().contains("scoop"))
        .unwrap_or(false)
}

/// Perform self-update.
///
/// Downloads the latest release from GitHub and replaces the current binary.
/// For Homebrew/Scoop installations, prints instructions instead.
pub fn self_update() -> Result<()> {
    // Check for package manager installations
    if is_homebrew_install() {
        println!("RocketIndex is installed via Homebrew.");
        println!("Run: brew upgrade rocket-tycoon/tap/rocket-index");
        return Ok(());
    }

    if is_scoop_install() {
        println!("RocketIndex is installed via Scoop.");
        println!("Run: scoop update rocketindex");
        return Ok(());
    }

    // Check if update is available
    println!("Checking for updates...");

    let latest = fetch_latest_version().context("Failed to check for updates")?;
    let _ = save_cache(&latest);

    let current = parse_version(CURRENT_VERSION);
    let latest_parsed = parse_version(&latest);

    match (current, latest_parsed) {
        (Some(curr), Some(lat)) if lat > curr => {
            println!("Updating from v{} to v{}...", CURRENT_VERSION, latest);
        }
        (Some(_), Some(_)) => {
            println!("Already up to date (v{})", CURRENT_VERSION);
            return Ok(());
        }
        _ => {
            println!("Could not parse version, attempting update anyway...");
        }
    }

    // Perform the update
    let status = self_update::backends::github::Update::configure()
        .repo_owner("rocket-tycoon")
        .repo_name("rocket-index")
        .bin_name("rkt")
        .show_download_progress(true)
        .current_version(CURRENT_VERSION)
        .build()
        .context("Failed to configure updater")?
        .update()
        .context("Failed to perform update")?;

    match status {
        self_update::Status::UpToDate(v) => {
            println!("Already up to date (v{})", v);
        }
        self_update::Status::Updated(v) => {
            println!("\x1b[32m✓ Updated to v{}\x1b[0m", v);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version_stable() {
        let v = parse_version("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_parse_version_prerelease() {
        let v = parse_version("0.1.0-beta.27").unwrap();
        assert_eq!(v.major, 0);
        assert_eq!(v.minor, 1);
        assert_eq!(v.patch, 0);
        assert!(!v.pre.is_empty());
    }

    #[test]
    fn test_version_comparison_prerelease() {
        let v1 = parse_version("0.1.0-beta.27").unwrap();
        let v2 = parse_version("0.1.0-beta.28").unwrap();
        assert!(v2 > v1);
    }

    #[test]
    fn test_version_comparison_stable_vs_prerelease() {
        let stable = parse_version("0.1.0").unwrap();
        let beta = parse_version("0.1.0-beta.28").unwrap();
        // Stable 0.1.0 is greater than 0.1.0-beta.28
        assert!(stable > beta);
    }

    #[test]
    fn test_version_comparison_major() {
        let v1 = parse_version("0.1.0-beta.27").unwrap();
        let v2 = parse_version("1.0.0").unwrap();
        assert!(v2 > v1);
    }

    #[test]
    fn test_current_version_parses() {
        // Ensure our current version string is valid
        let v = parse_version(CURRENT_VERSION);
        assert!(v.is_some(), "CURRENT_VERSION should be a valid semver");
    }

    #[test]
    fn test_cache_serialization() {
        let cache = VersionCache {
            latest_version: "0.1.0-beta.28".to_string(),
            checked_at: 1703500000,
        };
        let json = serde_json::to_string(&cache).unwrap();
        let parsed: VersionCache = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.latest_version, "0.1.0-beta.28");
        assert_eq!(parsed.checked_at, 1703500000);
    }

    #[test]
    fn test_github_release_deserialize() {
        let json = r#"{"tag_name": "v0.1.0-beta.28", "name": "Release"}"#;
        let release: GitHubRelease = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v0.1.0-beta.28");
    }

    #[test]
    fn test_github_releases_array_deserialize() {
        let json = r#"[{"tag_name": "v0.1.0-beta.28"}, {"tag_name": "v0.1.0-beta.27"}]"#;
        let releases: Vec<GitHubRelease> = serde_json::from_str(json).unwrap();
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].tag_name, "v0.1.0-beta.28");
    }

    #[test]
    fn test_strip_v_prefix() {
        let tag = "v0.1.0-beta.28";
        let version = tag.strip_prefix('v').unwrap_or(tag);
        assert_eq!(version, "0.1.0-beta.28");
    }

    #[test]
    fn test_is_homebrew_install_detection() {
        // Test the logic (not the actual path)
        let homebrew_path = "/opt/homebrew/Cellar/rocket-index/0.1.0/bin/rkt";
        assert!(homebrew_path.contains("homebrew") || homebrew_path.contains("Cellar"));

        let manual_path = "/usr/local/bin/rkt";
        assert!(!manual_path.contains("homebrew") && !manual_path.contains("Cellar"));

        let plugin_path = "/Users/test/.claude/plugins/cache/rocketindex/bin/rkt";
        assert!(!plugin_path.contains("homebrew") && !plugin_path.contains("Cellar"));
    }

    #[test]
    fn test_is_scoop_install_detection() {
        let scoop_path = r"C:\Users\test\scoop\apps\rocketindex\current\rkt.exe";
        assert!(scoop_path.contains("scoop"));

        let manual_path = r"C:\Program Files\rocketindex\rkt.exe";
        assert!(!manual_path.contains("scoop"));
    }
}
