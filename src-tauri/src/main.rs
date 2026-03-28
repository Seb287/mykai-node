// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod commands;
mod config;
mod heartbeat;
mod kaspad_manager;
mod rpc_client;

use commands::AppState;
use config::AppConfig;
use heartbeat::HeartbeatManager;
use kaspad_manager::KaspadManager;
use rpc_client::RpcClient;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() {
    // Initialize logging
    let log_dir = AppConfig::app_base_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let file_appender = tracing_appender::rolling::daily(&log_dir, "mykai-node.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    info!("MyKAI Node starting...");

    // Load configuration
    let config = AppConfig::load();
    config.ensure_dirs();
    let config = Arc::new(Mutex::new(config));

    // Create the kaspad manager, RPC client, and heartbeat manager
    let manager = Arc::new(KaspadManager::new(config.clone()));
    let rpc = Arc::new(RpcClient::new("ws://127.0.0.1:18110"));
    let heartbeat = Arc::new(HeartbeatManager::new(config.clone(), rpc.clone()));

    let state = AppState {
        config: config.clone(),
        manager: manager.clone(),
        rpc: rpc.clone(),
        heartbeat: heartbeat.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .setup(move |app| {
            // ── System Tray ─────────────────────────────────────────
            let show = MenuItem::with_id(app, "show", "Show MyKAI Node", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            let _tray = TrayIconBuilder::new()
                .tooltip("MyKAI Node")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        // Stop kaspad before quitting
                        let mgr = manager.clone();
                        let app_handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = mgr.stop().await;
                            app_handle.exit(0);
                        });
                    }
                    _ => {}
                })
                .build(app)?;

            // ── Window Close Behavior ───────────────────────────────
            // Minimize to tray instead of closing (keep node running)
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
            }

            // ── Auto-start Node ─────────────────────────────────────
            let config_clone = config.clone();
            let manager_for_autostart = app.state::<AppState>().manager.clone();
            let heartbeat_for_autostart = app.state::<AppState>().heartbeat.clone();
            tauri::async_runtime::spawn(async move {
                let cfg = config_clone.lock().await;
                let auto_start = cfg.auto_start_node;
                let kasmap_enabled = cfg.kasmap_enabled;
                drop(cfg);

                if auto_start {
                    if manager_for_autostart.is_installed().await {
                        info!("Auto-starting kaspad...");
                        if let Err(e) = manager_for_autostart.start().await {
                            tracing::error!("Auto-start failed: {}", e);
                        }
                    }
                }

                // Start KasMap heartbeat if enabled
                if kasmap_enabled {
                    info!("Auto-starting KasMap heartbeat...");
                    heartbeat_for_autostart.start().await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::is_installed,
            commands::is_running,
            commands::install_kaspad,
            commands::start_node,
            commands::stop_node,
            commands::check_update,
            commands::get_config,
            commands::set_auto_start_on_boot,
            commands::set_auto_start_node,
            commands::set_kasmap_token,
            commands::set_kasmap_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MyKAI Node");
}
