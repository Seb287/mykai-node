use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};

/// Application configuration persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Kaspad data directory (where blockchain data is stored).
    pub data_dir: PathBuf,

    /// Directory where the kaspad binary lives.
    pub bin_dir: PathBuf,

    /// Whether to start kaspad automatically when the app launches.
    pub auto_start_node: bool,

    /// Whether to launch the app on Windows login.
    pub auto_start_on_boot: bool,

    /// Currently installed kaspad version (e.g. "1.1.0").
    pub installed_version: Option<String>,

    /// wRPC-JSON endpoint for monitoring (default: ws://127.0.0.1:18110).
    pub wrpc_url: String,

    /// Target number of outbound peers.
    pub outbound_peers: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        let base = Self::app_base_dir();
        Self {
            data_dir: base.join("data"),
            bin_dir: base.join("bin"),
            auto_start_node: true,
            auto_start_on_boot: false,
            installed_version: None,
            wrpc_url: "ws://127.0.0.1:18110".into(),
            outbound_peers: 8,
        }
    }
}

impl AppConfig {
    /// Platform-specific base directory for MyKAI Node.
    /// Windows: %LOCALAPPDATA%\MyKAI Node
    /// Linux/macOS: ~/.local/share/mykai-node (unlikely but for dev)
    pub fn app_base_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MyKAI Node")
    }

    /// Path to the config file.
    fn config_path() -> PathBuf {
        Self::app_base_dir().join("config.json")
    }

    /// Load config from disk, or create default if it doesn't exist.
    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<AppConfig>(&content) {
                    Ok(config) => {
                        info!("Loaded config from {}", path.display());
                        return config;
                    }
                    Err(e) => {
                        warn!("Failed to parse config, using defaults: {}", e);
                    }
                },
                Err(e) => {
                    warn!("Failed to read config file, using defaults: {}", e);
                }
            }
        }
        let config = Self::default();
        config.save();
        config
    }

    /// Persist current config to disk.
    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    warn!("Failed to write config: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to serialize config: {}", e);
            }
        }
    }

    /// Full path to kaspad binary.
    pub fn kaspad_path(&self) -> PathBuf {
        if cfg!(windows) {
            self.bin_dir.join("kaspad.exe")
        } else {
            self.bin_dir.join("kaspad")
        }
    }

    /// Ensure all required directories exist.
    pub fn ensure_dirs(&self) {
        let _ = fs::create_dir_all(&self.data_dir);
        let _ = fs::create_dir_all(&self.bin_dir);
    }
}
