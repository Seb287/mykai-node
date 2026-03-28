use crate::config::AppConfig;
use crate::heartbeat::HeartbeatManager;
use crate::kaspad_manager::KaspadManager;
use crate::rpc_client::{NodeStatus, RpcClient};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Shared application state accessible from Tauri commands.
pub struct AppState {
    pub config: Arc<Mutex<AppConfig>>,
    pub manager: Arc<KaspadManager>,
    pub rpc: Arc<RpcClient>,
    pub heartbeat: Arc<HeartbeatManager>,
}

/// Response wrapper for all commands.
#[derive(Serialize)]
pub struct CommandResult<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T: Serialize> CommandResult<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

// ── Tauri Commands ──────────────────────────────────────────────────────

/// Get the current node status (sync state, peers, block height, etc).
#[tauri::command]
pub async fn get_status(state: tauri::State<'_, AppState>) -> Result<CommandResult<NodeStatus>, ()> {
    let is_running = state.manager.is_running().await;

    if !is_running {
        let mut status = NodeStatus::default();
        status.rpc_connected = false;
        return Ok(CommandResult::ok(status));
    }

    match state.rpc.get_node_status().await {
        Ok(status) => Ok(CommandResult::ok(status)),
        Err(_) => {
            // Node might be starting up, RPC not yet available
            let mut status = NodeStatus::default();
            status.rpc_connected = false;
            Ok(CommandResult::ok(status))
        }
    }
}

/// Check whether kaspad is installed.
#[tauri::command]
pub async fn is_installed(state: tauri::State<'_, AppState>) -> Result<CommandResult<bool>, ()> {
    Ok(CommandResult::ok(state.manager.is_installed().await))
}

/// Check whether kaspad process is running.
#[tauri::command]
pub async fn is_running(state: tauri::State<'_, AppState>) -> Result<CommandResult<bool>, ()> {
    Ok(CommandResult::ok(state.manager.is_running().await))
}

/// Download and install the latest kaspad release.
#[tauri::command]
pub async fn install_kaspad(
    state: tauri::State<'_, AppState>,
) -> Result<CommandResult<String>, ()> {
    info!("Install kaspad requested");
    match state.manager.download_latest().await {
        Ok(version) => Ok(CommandResult::ok(version)),
        Err(e) => Ok(CommandResult::err(e)),
    }
}

/// Start kaspad in private mode.
#[tauri::command]
pub async fn start_node(state: tauri::State<'_, AppState>) -> Result<CommandResult<()>, ()> {
    info!("Start node requested");
    match state.manager.start().await {
        Ok(()) => Ok(CommandResult::ok(())),
        Err(e) => Ok(CommandResult::err(e)),
    }
}

/// Stop kaspad.
#[tauri::command]
pub async fn stop_node(state: tauri::State<'_, AppState>) -> Result<CommandResult<()>, ()> {
    info!("Stop node requested");
    match state.manager.stop().await {
        Ok(()) => Ok(CommandResult::ok(())),
        Err(e) => Ok(CommandResult::err(e)),
    }
}

/// Check if a newer kaspad version is available.
#[tauri::command]
pub async fn check_update(
    state: tauri::State<'_, AppState>,
) -> Result<CommandResult<Option<String>>, ()> {
    match state.manager.check_for_update().await {
        Ok(version) => Ok(CommandResult::ok(version)),
        Err(e) => Ok(CommandResult::err(e)),
    }
}

/// Get the current app configuration.
#[tauri::command]
pub async fn get_config(state: tauri::State<'_, AppState>) -> Result<CommandResult<AppConfig>, ()> {
    let config = state.config.lock().await;
    Ok(CommandResult::ok(config.clone()))
}

/// Update a configuration setting.
#[tauri::command]
pub async fn set_auto_start_on_boot(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<CommandResult<()>, ()> {
    let mut config = state.config.lock().await;
    config.auto_start_on_boot = enabled;
    config.save();

    // Update Windows registry for auto-start
    #[cfg(windows)]
    {
        if let Err(e) = crate::autostart::set_auto_start(enabled) {
            return Ok(CommandResult::err(format!(
                "Failed to update auto-start: {}",
                e
            )));
        }
    }

    Ok(CommandResult::ok(()))
}

/// Update auto-start node setting (start kaspad when app launches).
#[tauri::command]
pub async fn set_auto_start_node(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<CommandResult<()>, ()> {
    let mut config = state.config.lock().await;
    config.auto_start_node = enabled;
    config.save();
    Ok(CommandResult::ok(()))
}

// ── KasMap Integration Commands ────────────────────────────────────

/// Set the KasMap node token and persist it.
#[tauri::command]
pub async fn set_kasmap_token(
    state: tauri::State<'_, AppState>,
    token: String,
) -> Result<CommandResult<()>, ()> {
    info!("KasMap token updated");
    let mut config = state.config.lock().await;

    if token.is_empty() {
        config.kasmap_token = None;
        config.kasmap_enabled = false;
    } else {
        config.kasmap_token = Some(token);
    }

    config.save();
    drop(config);

    // Restart heartbeat if it was enabled
    let cfg = state.config.lock().await;
    if cfg.kasmap_enabled && cfg.kasmap_token.is_some() {
        drop(cfg);
        state.heartbeat.start().await;
    } else {
        drop(cfg);
        state.heartbeat.stop().await;
    }

    Ok(CommandResult::ok(()))
}

/// Enable or disable KasMap reporting.
#[tauri::command]
pub async fn set_kasmap_enabled(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<CommandResult<()>, ()> {
    info!("KasMap reporting: {}", if enabled { "enabled" } else { "disabled" });
    let mut config = state.config.lock().await;
    config.kasmap_enabled = enabled;
    config.save();

    let has_token = config.kasmap_token.is_some();
    drop(config);

    if enabled && has_token {
        state.heartbeat.start().await;
    } else {
        state.heartbeat.stop().await;
    }

    Ok(CommandResult::ok(()))
}
