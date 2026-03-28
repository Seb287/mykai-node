use crate::config::AppConfig;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Manages the kaspad process lifecycle: download, start, stop, update.
pub struct KaspadManager {
    config: Arc<Mutex<AppConfig>>,
    process: Arc<Mutex<Option<Child>>>,
}

/// GitHub release asset metadata.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

impl KaspadManager {
    pub fn new(config: Arc<Mutex<AppConfig>>) -> Self {
        Self {
            config,
            process: Arc::new(Mutex::new(None)),
        }
    }

    /// Check if kaspad binary exists on disk.
    pub async fn is_installed(&self) -> bool {
        let config = self.config.lock().await;
        config.kaspad_path().exists()
    }

    /// Check if kaspad process is currently running.
    pub async fn is_running(&self) -> bool {
        let mut proc = self.process.lock().await;
        if let Some(child) = proc.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    *proc = None;
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    *proc = None;
                    false
                }
            }
        } else {
            false
        }
    }

    /// Try to extract a SHA256 hash for an asset from the release body text.
    /// Many projects include checksums in the release notes, e.g.:
    /// `SHA256: abcdef1234... filename.zip`
    /// or a SHA256SUMS-style format: `abcdef1234...  filename.zip`
    fn extract_expected_hash(release_body: &Option<String>, asset_name: &str) -> Option<String> {
        let body = release_body.as_ref()?;
        for line in body.lines() {
            let line = line.trim();
            // Skip empty lines
            if line.is_empty() {
                continue;
            }
            // Check if this line mentions our asset filename
            if !line.contains(asset_name) {
                continue;
            }
            // Try to find a 64-char hex string (SHA256) on this line
            for word in line.split_whitespace() {
                let clean = word.trim_matches(|c: char| !c.is_ascii_hexdigit());
                if clean.len() == 64 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(clean.to_lowercase());
                }
            }
        }
        None
    }

    /// Download the latest kaspad release from GitHub.
    /// Returns the version string that was downloaded.
    pub async fn download_latest(&self) -> Result<String, String> {
        info!("Fetching latest kaspad release from GitHub...");

        let client = reqwest::Client::builder()
            .user_agent("MyKAI-Node/0.1")
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        // Get latest release info (M-1: check HTTP status before parsing)
        let response = client
            .get("https://api.github.com/repos/kaspanet/rusty-kaspa/releases/latest")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch release info: {}", e))?;

        if response.status() == 403 {
            return Err("GitHub API rate limit exceeded. Please try again later.".into());
        }
        if !response.status().is_success() {
            return Err(format!("GitHub API error: {}", response.status()));
        }

        let release: GitHubRelease = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse release info: {}", e))?;

        let version = release.tag_name.trim_start_matches('v').to_string();
        info!("Latest kaspad version: {}", version);

        // Find the Windows x64 asset
        let asset = release
            .assets
            .iter()
            .find(|a| {
                let name = a.name.to_lowercase();
                name.contains("windows")
                    && (name.contains("x64") || name.contains("x86_64"))
                    && name.ends_with(".zip")
            })
            .ok_or_else(|| {
                let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
                format!(
                    "No Windows x64 binary found in release assets. Available: {:?}",
                    names
                )
            })?;

        info!("Downloading: {}", asset.name);

        // Download the zip (M-1: check HTTP status)
        let dl_response = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .map_err(|e| format!("Failed to download: {}", e))?;

        if !dl_response.status().is_success() {
            return Err(format!("Download failed: HTTP {}", dl_response.status()));
        }

        let bytes = dl_response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read download: {}", e))?;

        info!("Downloaded {} bytes", bytes.len());

        // C-1: Verify SHA256 checksum if available in release notes
        let computed_hash = {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            format!("{:x}", hasher.finalize())
        };

        if let Some(expected_hash) = Self::extract_expected_hash(&release.body, &asset.name) {
            if computed_hash != expected_hash {
                return Err(format!(
                    "Integrity check FAILED! Expected SHA256: {}, got: {}. Download may be corrupted or tampered with.",
                    expected_hash, computed_hash
                ));
            }
            info!("SHA256 checksum verified: {}", &computed_hash[..16]);
        } else {
            warn!(
                "No SHA256 checksum found in release notes for {}. Skipping verification. Hash: {}",
                asset.name, &computed_hash[..16]
            );
        }

        info!("Extracting...");

        // C-2: Safe zip extraction with enclosed_name() and exact filename match
        let config = self.config.lock().await;
        config.ensure_dirs();
        let bin_dir = config.bin_dir.clone();
        drop(config);

        let cursor = Cursor::new(bytes.to_vec());
        let mut archive =
            zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open zip: {}", e))?;

        let mut found_kaspad = false;
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| format!("Failed to read zip entry: {}", e))?;

            // C-2: Use enclosed_name() for safe path handling
            let safe_path = match file.enclosed_name() {
                Some(p) => p.to_owned(),
                None => {
                    warn!("Skipping unsafe zip entry: {}", file.name());
                    continue;
                }
            };

            // Only extract regular files (not symlinks, not directories)
            if !file.is_file() {
                continue;
            }

            // C-2: Exact filename match only (not ends_with)
            let file_name = safe_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if file_name == "kaspad.exe" || file_name == "kaspad" {
                let out_path = if cfg!(windows) {
                    bin_dir.join("kaspad.exe")
                } else {
                    bin_dir.join("kaspad")
                };

                let mut outfile = std::fs::File::create(&out_path)
                    .map_err(|e| format!("Failed to create file: {}", e))?;

                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract: {}", e))?;

                // Set executable permission on Unix (for development)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        &out_path,
                        std::fs::Permissions::from_mode(0o755),
                    );
                }

                info!("Extracted kaspad to {}", out_path.display());
                found_kaspad = true;
            }
        }

        if !found_kaspad {
            return Err("kaspad binary not found in the downloaded archive".into());
        }

        // Update config with installed version
        let mut config = self.config.lock().await;
        config.installed_version = Some(version.clone());
        config.save();

        info!("kaspad {} installed successfully", version);
        Ok(version)
    }

    /// Start the kaspad process in private mode (no inbound peers).
    /// M-4: Holds the process lock for the entire operation to prevent TOCTOU race.
    pub async fn start(&self) -> Result<(), String> {
        let mut proc = self.process.lock().await;

        // Check if already running (within the same lock scope)
        if let Some(child) = proc.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process exited, clear it
                    *proc = None;
                }
                Ok(None) => {
                    return Err("kaspad is already running".into());
                }
                Err(_) => {
                    *proc = None;
                }
            }
        }

        let config = self.config.lock().await;
        let kaspad_path = config.kaspad_path();
        let data_dir = config.data_dir.clone();
        let outbound_peers = config.outbound_peers;
        drop(config);

        if !kaspad_path.exists() {
            return Err("kaspad binary not found. Please install first.".into());
        }

        info!("Starting kaspad (private mode)...");

        let mut child = Command::new(&kaspad_path)
            .arg("--utxoindex")
            .arg("--maxinpeers=0")
            .arg(format!("--outpeers={}", outbound_peers))
            .arg(format!("--appdir={}", data_dir.display()))
            .arg("--rpclisten-json=127.0.0.1:18110")
            .arg("--yes")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start kaspad: {}", e))?;

        info!("kaspad started with PID: {:?}", child.id());

        // Brief delay then verify the process didn't crash immediately
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "kaspad exited immediately with status: {}. Check logs in the data directory.",
                    status
                ));
            }
            Err(e) => {
                return Err(format!("Failed to check kaspad status: {}", e));
            }
            Ok(None) => {} // Still running, good
        }

        // Store the process handle (still within the same lock scope)
        *proc = Some(child);

        Ok(())
    }

    /// Stop the kaspad process.
    /// H-4: Attempt graceful shutdown first (taskkill without /F on Windows),
    /// then force-kill as fallback.
    pub async fn stop(&self) -> Result<(), String> {
        let mut proc = self.process.lock().await;
        if let Some(mut child) = proc.take() {
            info!("Stopping kaspad...");

            let pid = child.id();

            // H-4: Try graceful shutdown first
            #[cfg(windows)]
            {
                if let Some(pid) = pid {
                    // taskkill without /F sends WM_CLOSE, which kaspad can handle gracefully
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn();
                }
            }

            #[cfg(not(windows))]
            {
                // On Unix, send SIGTERM for graceful shutdown
                let _ = child.start_kill();
            }

            // Wait for clean exit, with a timeout
            tokio::select! {
                _ = child.wait() => {
                    info!("kaspad exited cleanly");
                },
                _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
                    warn!("kaspad did not stop within 15s, force killing");
                    let _ = child.kill().await;
                }
            }

            info!("kaspad stopped");
            Ok(())
        } else {
            Err("kaspad is not running".into())
        }
    }

    /// Check if a newer version is available on GitHub.
    pub async fn check_for_update(&self) -> Result<Option<String>, String> {
        let client = reqwest::Client::builder()
            .user_agent("MyKAI-Node/0.1")
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?;

        // M-1: Check HTTP status before parsing
        let response = client
            .get("https://api.github.com/repos/kaspanet/rusty-kaspa/releases/latest")
            .send()
            .await
            .map_err(|e| format!("Failed to check for updates: {}", e))?;

        if response.status() == 403 {
            return Err("GitHub API rate limit exceeded. Please try again later.".into());
        }
        if !response.status().is_success() {
            return Err(format!("GitHub API error: {}", response.status()));
        }

        let release: GitHubRelease = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse release: {}", e))?;

        let latest = release.tag_name.trim_start_matches('v').to_string();
        let config = self.config.lock().await;

        if let Some(ref installed) = config.installed_version {
            if installed != &latest {
                Ok(Some(latest))
            } else {
                Ok(None)
            }
        } else {
            Ok(Some(latest))
        }
    }
}
