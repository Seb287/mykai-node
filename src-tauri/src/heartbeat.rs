use crate::config::AppConfig;
use crate::rpc_client::RpcClient;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Supabase project configuration for kasmap.org heartbeat reporting.
/// These are public values (same as what's in the kasmap.org frontend).
const SUPABASE_URL: &str = "https://yoxtiqcndeetuzqrhgby.supabase.co";
const SUPABASE_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6InlveHRpcWNuZGVldHV6cXJoZ2J5Iiwicm9sZSI6ImFub24iLCJpYXQiOjE3NjI4NjcwMTUsImV4cCI6MjA3ODQ0MzAxNX0.UM8wOnK2HXiwx7wxV8O4U5ulVZLZEk-tet8CyS7RAWQ";

/// Heartbeat payload sent to the Supabase RPC function.
#[derive(Debug, Serialize)]
struct HeartbeatPayload {
    p_token: String,
    p_node_status: String,
    p_sync_progress: f64,
    p_block_count: i64,
    p_peer_count: i32,
    p_node_type: String,
    p_kaspad_version: String,
    p_client_name: String,
    p_client_version: String,
    p_network: String,
}

/// Result from the heartbeat RPC call.
#[derive(Debug, serde::Deserialize)]
struct HeartbeatResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default = "default_next_ping")]
    next_ping_seconds: u64,
}

fn default_next_ping() -> u64 {
    1800
}

/// Manages the background heartbeat loop that reports node status to kasmap.org.
pub struct HeartbeatManager {
    config: Arc<Mutex<AppConfig>>,
    rpc: Arc<RpcClient>,
    http_client: reqwest::Client,
    /// Handle to the background task so we can cancel it.
    task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl HeartbeatManager {
    pub fn new(config: Arc<Mutex<AppConfig>>, rpc: Arc<RpcClient>) -> Self {
        let http_client = reqwest::Client::builder()
            .user_agent("MyKAI-Node/0.1")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client for heartbeat");

        Self {
            config,
            rpc,
            http_client,
            task_handle: Mutex::new(None),
        }
    }

    /// Start the heartbeat background loop. Safe to call multiple times —
    /// it will stop any existing loop first.
    pub async fn start(&self) {
        self.stop().await;

        let config = self.config.clone();
        let rpc = self.rpc.clone();
        let client = self.http_client.clone();

        let handle = tokio::spawn(async move {
            info!("KasMap heartbeat started");

            // Initial interval: 30 minutes (server can adjust via next_ping_seconds)
            let mut interval_secs: u64 = 1800;

            loop {
                // Check if still enabled and has a token
                let cfg = config.lock().await;
                let enabled = cfg.kasmap_enabled;
                let token = cfg.kasmap_token.clone();
                drop(cfg);

                if !enabled {
                    debug!("KasMap heartbeat disabled, stopping loop");
                    break;
                }

                if let Some(ref token) = token {
                    match Self::send_heartbeat(&client, &rpc, token).await {
                        Ok(response) => {
                            if response.ok {
                                debug!(
                                    "Heartbeat sent successfully, next ping in {}s",
                                    response.next_ping_seconds
                                );
                                interval_secs = response.next_ping_seconds;
                            } else {
                                warn!(
                                    "Heartbeat rejected: {}",
                                    response.error.unwrap_or_default()
                                );
                            }
                        }
                        Err(e) => {
                            warn!("Heartbeat failed: {}", e);
                            // On error, keep retrying with a longer interval
                        }
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
            }
        });

        let mut task = self.task_handle.lock().await;
        *task = Some(handle);
    }

    /// Stop the heartbeat background loop.
    pub async fn stop(&self) {
        let mut task = self.task_handle.lock().await;
        if let Some(handle) = task.take() {
            handle.abort();
            info!("KasMap heartbeat stopped");
        }
    }

    /// Send a single heartbeat to kasmap.org via Supabase RPC.
    async fn send_heartbeat(
        client: &reqwest::Client,
        rpc: &RpcClient,
        token: &str,
    ) -> Result<HeartbeatResponse, String> {
        // Get current node status
        let (node_status, sync_progress, block_count, peer_count, kaspad_version, network) =
            match rpc.get_node_status().await {
                Ok(status) => {
                    let ns = if status.is_synced {
                        "online"
                    } else if status.rpc_connected {
                        "syncing"
                    } else {
                        "offline"
                    };
                    (
                        ns.to_string(),
                        status.sync_progress * 100.0,
                        status.block_count as i64,
                        status.peer_count as i32,
                        status.server_version.clone(),
                        status.network.clone(),
                    )
                }
                Err(_) => {
                    // Node not reachable — still send heartbeat so kasmap knows we're alive
                    ("offline".to_string(), 0.0, 0, 0, String::new(), "mainnet".to_string())
                }
            };

        let payload = HeartbeatPayload {
            p_token: token.to_string(),
            p_node_status: node_status,
            p_sync_progress: sync_progress,
            p_block_count: block_count,
            p_peer_count: peer_count,
            p_node_type: "private".to_string(),
            p_kaspad_version: kaspad_version,
            p_client_name: "mykai-node".to_string(),
            p_client_version: env!("CARGO_PKG_VERSION").to_string(),
            p_network: if network.is_empty() {
                "mainnet".to_string()
            } else {
                network
            },
        };

        let url = format!("{}/rest/v1/rpc/node_heartbeat", SUPABASE_URL);

        let resp = client
            .post(&url)
            .header("apikey", SUPABASE_ANON_KEY)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}: {}", resp.status(), resp.text().await.unwrap_or_default()));
        }

        resp.json::<HeartbeatResponse>()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))
    }
}
