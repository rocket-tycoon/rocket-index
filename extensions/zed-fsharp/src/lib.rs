//! Zed extension for F# language support.
//!
//! This extension provides:
//! - Syntax highlighting via tree-sitter-fsharp
//! - Language server integration via fsharp-lsp
//!
//! ## Local Development
//!
//! Set the `FSHARP_LSP_PATH` environment variable to use a local binary:
//! ```bash
//! export FSHARP_LSP_PATH="/path/to/fsharp-tools/target/release/fsharp-lsp"
//! ```
//!
//! Then restart Zed. The extension will use your local binary instead of
//! downloading from GitHub.

use std::fs;
use zed_extension_api::{self as zed, Result};

struct FSharpExtension {
    cached_binary_path: Option<String>,
}

impl zed::Extension for FSharpExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let binary_path = self.ensure_binary(worktree)?;

        Ok(zed::Command {
            command: binary_path,
            args: vec![],
            env: worktree.shell_env(),
        })
    }
}

impl FSharpExtension {
    /// Ensure the fsharp-lsp binary is available.
    ///
    /// Resolution order:
    /// 1. FSHARP_LSP_PATH environment variable (for local development)
    /// 2. Download from GitHub releases (for production)
    fn ensure_binary(&mut self, worktree: &zed::Worktree) -> Result<String> {
        // Return cached path if available and valid
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok() {
                return Ok(path.clone());
            }
        }

        // Check for local development override
        if let Some(path) = self.check_local_binary(worktree) {
            self.cached_binary_path = Some(path.clone());
            return Ok(path);
        }

        // Fall back to downloading from GitHub
        self.download_binary()
    }

    /// Check for a local binary via FSHARP_LSP_PATH environment variable or known locations.
    fn check_local_binary(&self, worktree: &zed::Worktree) -> Option<String> {
        let env = worktree.shell_env();

        // Check FSHARP_LSP_PATH environment variable first
        for (key, value) in &env {
            if key == "FSHARP_LSP_PATH" && !value.is_empty() {
                eprintln!("fsharp-lsp: using FSHARP_LSP_PATH={}", value);
                return Some(value.clone());
            }
        }

        // Check if we're in the fsharp-tools project and construct path to binary
        let worktree_root = worktree.root_path();
        eprintln!("fsharp-lsp: worktree root is {}", worktree_root);

        // For development: look for binary in fsharp-tools/target/release
        if worktree_root.contains("fsharp-tools") {
            // Find the fsharp-tools root by looking for it in the path
            if let Some(idx) = worktree_root.find("fsharp-tools") {
                let fsharp_tools_root = &worktree_root[..idx + "fsharp-tools".len()];
                let binary_path = format!("{}/target/release/fsharp-lsp", fsharp_tools_root);
                eprintln!("fsharp-lsp: trying development path {}", binary_path);
                return Some(binary_path);
            }
        }

        None
    }

    /// Download the fsharp-lsp binary from GitHub releases.
    fn download_binary(&mut self) -> Result<String> {
        // Determine platform and architecture
        let (platform, arch) = zed::current_platform();

        let platform_str = match platform {
            zed::Os::Mac => "apple-darwin",
            zed::Os::Linux => "unknown-linux-gnu",
            zed::Os::Windows => "pc-windows-msvc",
        };

        let arch_str = match arch {
            zed::Architecture::Aarch64 => "aarch64",
            zed::Architecture::X86 => "x86",
            zed::Architecture::X8664 => "x86_64",
        };

        // Get the latest release from GitHub
        let release = zed::latest_github_release(
            "yourname/fsharp-tools",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        // Find the appropriate asset for this platform
        let asset_name = format!("fsharp-lsp-{}-{}.tar.gz", arch_str, platform_str);
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| format!("No binary available for {}-{}", arch_str, platform_str))?;

        // Set up paths
        let version_dir = format!("fsharp-lsp-{}", release.version);
        let binary_path = format!("{}/fsharp-lsp", version_dir);

        // Check if we need to download
        if fs::metadata(&binary_path).is_err() {
            // Download and extract the binary
            zed::download_file(
                &asset.download_url,
                &version_dir,
                zed::DownloadedFileType::GzipTar,
            )
            .map_err(|e| format!("Failed to download fsharp-lsp: {}", e))?;

            // Make the binary executable
            zed::make_file_executable(&binary_path)
                .map_err(|e| format!("Failed to make binary executable: {}", e))?;
        }

        // Cache and return the path
        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

zed::register_extension!(FSharpExtension);
