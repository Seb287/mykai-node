use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, trace};

/// Node status information gathered from kaspad RPC.
#[derive(Debug, Clone, Serialize, Default)]
pub struct NodeStatus {
    /// Whether the node is fully synced with the network.
    pub is_synced: bool,
    /// Server version string.
    pub server_version: String,
    /// Current network (mainnet, testnet, etc).
    pub network: String,
    /// Whether UTXO index is enabled.
    pub has_utxo_index: bool,
    /// Number of blocks in the DAG.
    pub block_count: u64,
    /// Number of headers received.
    pub header_count: u64,
    /// Current DAA (Difficulty Adjustment Algorithm) score.
    pub virtual_daa_score: u64,
    /// Number of connected peers (outbound only in private mode).
    pub peer_count: u32,
    /// List of connected peer addresses.
    pub peers: Vec<PeerInfo>,
    /// Whether RPC connection is active.
    pub rpc_connected: bool,
    /// Estimated sync progress (0.0 - 1.0).
    pub sync_progress: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerInfo {
    pub address: String,
    pub is_outbound: bool,
    pub user_agent: String,
}

/// Client for kaspad's wRPC-JSON interface (WebSocket on port 18110).
pub struct RpcClient {
    url: String,
    request_id: AtomicU64,
}

impl RpcClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            request_id: AtomicU64::new(1),
        }
    }

    /// Fetch a complete node status snapshot by calling multiple RPC methods.
    pub async fn get_node_status(&self) -> Result<NodeStatus, String> {
        let mut status = NodeStatus::default();

        // Connect to websocket with timeout
        let (mut ws, _) = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            connect_async(&self.url),
        )
        .await
        .map_err(|_| "WebSocket connection timed out".to_string())?
        .map_err(|e| format!("WebSocket connection failed: {}", e))?;

        // GetInfo - basic node info
        let info_req = self.build_request("getInfoRequest", json!({}));
        ws.send(Message::Text(info_req))
            .await
            .map_err(|e| format!("Send failed: {}", e))?;

        if let Some(resp) = self.read_response(&mut ws).await? {
            if let Some(info) = resp.get("getInfoResponse") {
                status.is_synced = info.get("isSynced").and_then(|v| v.as_bool()).unwrap_or(false);
                status.server_version = info
                    .get("serverVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                status.network = info
                    .get("networkId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                status.has_utxo_index = info
                    .get("hasUtxoIndex")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
            }
        }

        // GetBlockDagInfo - DAG state
        let dag_req = self.build_request("getBlockDagInfoRequest", json!({}));
        ws.send(Message::Text(dag_req))
            .await
            .map_err(|e| format!("Send failed: {}", e))?;

        if let Some(resp) = self.read_response(&mut ws).await? {
            if let Some(dag) = resp.get("getBlockDagInfoResponse") {
                status.block_count = dag
                    .get("blockCount")
                    .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64()))
                    .unwrap_or(0);
                status.header_count = dag
                    .get("headerCount")
                    .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64()))
                    .unwrap_or(0);
                status.virtual_daa_score = dag
                    .get("virtualDaaScore")
                    .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64()))
                    .unwrap_or(0);
            }
        }

        // GetConnectedPeerInfo - peer list
        let peer_req = self.build_request("getConnectedPeerInfoRequest", json!({}));
        ws.send(Message::Text(peer_req))
            .await
            .map_err(|e| format!("Send failed: {}", e))?;

        if let Some(resp) = self.read_response(&mut ws).await? {
            if let Some(peer_resp) = resp.get("getConnectedPeerInfoResponse") {
                if let Some(infos) = peer_resp.get("infos").and_then(|v| v.as_array()) {
                    status.peers = infos
                        .iter()
                        .filter_map(|p| {
                            Some(PeerInfo {
                                address: p.get("address")?.as_str()?.to_string(),
                                is_outbound: p.get("isOutbound").and_then(|v| v.as_bool()).unwrap_or(false),
                                user_agent: p
                                    .get("userAgent")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            })
                        })
                        .collect();
                    status.peer_count = status.peers.len() as u32;
                }
            }
        }

        // Close websocket
        let _ = ws.close(None).await;

        // Calculate sync progress estimate
        // During IBD, header_count advances ahead of block_count
        // When synced, they should be approximately equal
        status.sync_progress = if status.is_synced {
            1.0
        } else if status.header_count > 0 {
            // Use block_count / header_count as a rough progress indicator
            // This isn't perfectly accurate but gives a useful signal
            let ratio = status.block_count as f64 / status.header_count as f64;
            ratio.min(0.99) // Cap at 99% until is_synced is true
        } else {
            0.0
        };

        status.rpc_connected = true;

        debug!(
            "Node status: synced={}, blocks={}, headers={}, peers={}, daa={}",
            status.is_synced, status.block_count, status.header_count,
            status.peer_count, status.virtual_daa_score
        );

        Ok(status)
    }

    /// Build a wRPC-JSON request message.
    /// Kaspa wRPC-JSON uses a specific envelope format.
    fn build_request(&self, method: &str, params: Value) -> String {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "id": id,
            method: params
        });
        request.to_string()
    }

    /// Read and parse a single WebSocket response.
    async fn read_response(
        &self,
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Result<Option<Value>, String> {
        // Read with a timeout to avoid hanging
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await;

        match result {
            Ok(Some(Ok(Message::Text(text)))) => {
                trace!("RPC response: {}", text);
                let value: Value =
                    serde_json::from_str(&text).map_err(|e| format!("JSON parse error: {}", e))?;
                Ok(Some(value))
            }
            Ok(Some(Ok(_))) => Ok(None), // Non-text message, skip
            Ok(Some(Err(e))) => Err(format!("WebSocket error: {}", e)),
            Ok(None) => Err("WebSocket stream ended".into()),
            Err(_) => Err("RPC response timeout".into()),
        }
    }

    /// Simple connectivity check - just try to connect and get basic info.
    pub async fn ping(&self) -> bool {
        match connect_async(&self.url).await {
            Ok((mut ws, _)) => {
                let _ = ws.close(None).await;
                true
            }
            Err(_) => false,
        }
    }
}
