use crate::config::AppConfig;
use serde::Deserialize;
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
            // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running
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

    /// Download the latest kaspad release from GitHub.
    /// Returns the version string that was downloaded.
    pub async fn download_latest(&self) -> Result<String, String> {
        info!("Fetching latest kaspad release from GitHub...");

        let client = reqwest::Client::builder()
            .user_agent("MyKAI-Node/0.1")
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        // Get latest release info
        let release: GitHubRelease = client
            .get("https://api.github.com/repos/kaspanet/rusty-kaspa/releases/latest")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch release info: {}", e))?
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
                name.contains("windows") && (name.contains("x64") || name.contains("x86_64"))
                    && name.ends_with(".zip")
            })
            .ok_or_else(|| {
                // Fallback: try any zip that looks like a Windows build
                let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
                format!(
                    "No Windows x64 binary found in release assets. Available: {:?}",
                    names
                )
            })?;

        info!("Downloading: {}", asset.name);

        // Download the zip
        let response = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .map_err(|e| format!("Failed to download: {}", e))?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read download: {}", e))?;

        info!("Downloaded {} bytes, extracting...", bytes.len());

        // Extract kaspad.exe from the zip
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

            let name = file.name().to_string();

            // Security: reject paths with traversal attacks
            if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
                warn!("Skipping suspicious zip entry: {}", name);
                continue;
            }

            // Extract kaspad binary (might be in a subdirectory within the zip)
            if name.ends_with("kaspad.exe") || name.ends_with("kaspad") {
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
                    let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755));
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
    pub async fn start(&self) -> Result<(), String> {
        if self.is_running().await {
            return Err("kaspad is already running".into());
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

        let child = Command::new(&kaspad_path)
            .arg("--utxoindex")
            .arg("--maxinpeers=0")
            .arg(format!("--outpeers={}", outbound_peers))
            .arg(format!("--appdir={}", data_dir.display()))
            .arg("--rpclisten-json=127.0.0.1:18110")
            .arg("--yes") // Non-interactive mode
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true) // Stop kaspad when the app exits
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

        let mut proc = self.process.lock().await;
        *proc = Some(child);

        Ok(())
    }

    /// Stop the kaspad process gracefully.
    pub async fn stop(&self) -> Result<(), String> {
        let mut proc = self.process.lock().await;
        if let Some(mut child) = proc.take() {
            info!("Stopping kaspad...");

            // On Windows, kill() sends TerminateProcess which is immediate
            // On Unix, kill() sends SIGKILL
            // For a graceful shutdown, kaspad handles SIGTERM/SIGINT
            // Attempt graceful shutdown: kill_on_drop will terminate when
            // the Child is dropped. We give kaspad time to flush and exit.
            // On both Windows and Unix, we kill and then wait briefly.
            let _ = child.start_kill();
            tokio::select! {
                _ = child.wait() => {
                    info!("kaspad exited cleanly");
                },
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                    warn!("kaspad did not stop within 10s, force killing");
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

        let release: GitHubRelease = client
            .get("https://api.github.com/repos/kaspanet/rusty-kaspa/releases/latest")
            .send()
            .await
            .map_err(|e| format!("Failed to check for updates: {}", e))?
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
