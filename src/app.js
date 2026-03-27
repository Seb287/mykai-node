// ── MyKAI Node — Frontend Application ───────────────────────────────
// Communicates with the Tauri Rust backend via invoke().

const { invoke } = window.__TAURI__.core;

// ── State ───────────────────────────────────────────────────────────

let isRunning = false;
let pollInterval = null;

// ── DOM Elements ────────────────────────────────────────────────────

const $ = (sel) => document.querySelector(sel);

const els = {
  setupScreen: $("#setup-screen"),
  dashboardScreen: $("#dashboard-screen"),
  installBtn: $("#install-btn"),
  installStatus: $("#install-status"),
  statusCard: $("#status-card"),
  statusLabel: $("#status-label"),
  statusDetail: $("#status-detail"),
  syncSection: $("#sync-section"),
  progressBar: $("#progress-bar"),
  syncPercent: $("#sync-percent"),
  syncBlocks: $("#sync-blocks"),
  statBlocks: $("#stat-blocks"),
  statDaa: $("#stat-daa"),
  statPeers: $("#stat-peers"),
  statVersion: $("#stat-version"),
  toggleBtn: $("#toggle-btn"),
  toggleLabel: $("#toggle-label"),
  playIcon: $("#play-icon"),
  stopIcon: $("#stop-icon"),
  networkCard: $("#network-card"),
  infoNetwork: $("#info-network"),
  infoUtxo: $("#info-utxo"),
  settingsBtn: $("#settings-btn"),
  settingsPanel: $("#settings-panel"),
  settingsClose: $("#settings-close"),
  settingAutostartNode: $("#setting-autostart-node"),
  settingAutostartBoot: $("#setting-autostart-boot"),
  checkUpdateBtn: $("#check-update-btn"),
  updateStatus: $("#update-status"),
  kaspadVersion: $("#kaspad-version"),
};

// ── Initialization ──────────────────────────────────────────────────

async function init() {
  try {
    const result = await invoke("is_installed");
    if (result.success && result.data) {
      showDashboard();
      // Check if node is already running and start polling
      const runResult = await invoke("is_running");
      isRunning = runResult.success && runResult.data;
      updateToggleButton();
      startPolling();
    } else {
      showSetup();
    }

    // Load settings
    const configResult = await invoke("get_config");
    if (configResult.success && configResult.data) {
      els.settingAutostartNode.checked = configResult.data.auto_start_node;
      els.settingAutostartBoot.checked = configResult.data.auto_start_on_boot;
      if (configResult.data.installed_version) {
        els.kaspadVersion.textContent = `kaspad: v${configResult.data.installed_version}`;
      }
    }
  } catch (e) {
    console.error("Init error:", e);
    showSetup();
  }

  setupEventListeners();
}

// ── Screen Management ───────────────────────────────────────────────

function showSetup() {
  els.setupScreen.style.display = "flex";
  els.dashboardScreen.style.display = "none";
}

function showDashboard() {
  els.setupScreen.style.display = "none";
  els.dashboardScreen.style.display = "block";
}

// ── Event Listeners ─────────────────────────────────────────────────

function setupEventListeners() {
  // Install button
  els.installBtn.addEventListener("click", handleInstall);

  // Start/Stop toggle
  els.toggleBtn.addEventListener("click", handleToggle);

  // Settings
  els.settingsBtn.addEventListener("click", () => {
    els.settingsPanel.style.display = "block";
  });
  els.settingsClose.addEventListener("click", () => {
    els.settingsPanel.style.display = "none";
  });

  // Setting toggles
  els.settingAutostartNode.addEventListener("change", async (e) => {
    await invoke("set_auto_start_node", { enabled: e.target.checked });
  });
  els.settingAutostartBoot.addEventListener("change", async (e) => {
    await invoke("set_auto_start_on_boot", { enabled: e.target.checked });
  });

  // Check for updates
  els.checkUpdateBtn.addEventListener("click", handleCheckUpdate);
}

// ── Install Handler ─────────────────────────────────────────────────

async function handleInstall() {
  els.installBtn.disabled = true;
  els.installBtn.querySelector(".btn-text").textContent = "Downloading...";
  els.installBtn.querySelector(".btn-spinner").style.display = "inline-block";
  els.installStatus.textContent = "Downloading kaspad from GitHub...";

  try {
    const result = await invoke("install_kaspad");
    if (result.success) {
      els.installStatus.textContent = `kaspad v${result.data} installed! Starting node...`;
      els.kaspadVersion.textContent = `kaspad: v${result.data}`;

      // Auto-start after install
      const startResult = await invoke("start_node");
      if (startResult.success) {
        isRunning = true;
        showDashboard();
        updateToggleButton();
        startPolling();
      } else {
        els.installStatus.textContent = `Installed, but failed to start: ${startResult.error}`;
        showDashboard();
      }
    } else {
      els.installStatus.textContent = `Error: ${result.error}`;
      els.installStatus.style.color = "#F85149";
    }
  } catch (e) {
    els.installStatus.textContent = `Error: ${e}`;
    els.installStatus.style.color = "#F85149";
  } finally {
    els.installBtn.disabled = false;
    els.installBtn.querySelector(".btn-text").textContent = "Install & Start Node";
    els.installBtn.querySelector(".btn-spinner").style.display = "none";
  }
}

// ── Toggle Node Start/Stop ──────────────────────────────────────────

async function handleToggle() {
  els.toggleBtn.disabled = true;

  try {
    if (isRunning) {
      const result = await invoke("stop_node");
      if (result.success) {
        isRunning = false;
      }
    } else {
      const result = await invoke("start_node");
      if (result.success) {
        isRunning = true;
      }
    }
  } catch (e) {
    console.error("Toggle error:", e);
  }

  els.toggleBtn.disabled = false;
  updateToggleButton();
}

function updateToggleButton() {
  if (isRunning) {
    els.toggleBtn.className = "toggle-btn running";
    els.toggleLabel.textContent = "Stop Node";
    els.playIcon.style.display = "none";
    els.stopIcon.style.display = "inline";
  } else {
    els.toggleBtn.className = "toggle-btn stopped";
    els.toggleLabel.textContent = "Start Node";
    els.playIcon.style.display = "inline";
    els.stopIcon.style.display = "none";
  }
}

// ── Status Polling ──────────────────────────────────────────────────

function startPolling() {
  // Poll immediately, then every 3 seconds
  pollStatus();
  if (pollInterval) clearInterval(pollInterval);
  pollInterval = setInterval(pollStatus, 3000);
}

async function pollStatus() {
  try {
    const result = await invoke("get_status");
    if (result.success) {
      updateDashboard(result.data);
    }
  } catch (e) {
    // Ignore polling errors silently
  }

  // Also check if process is still running
  try {
    const runResult = await invoke("is_running");
    if (runResult.success) {
      const wasRunning = isRunning;
      isRunning = runResult.data;
      if (wasRunning !== isRunning) {
        updateToggleButton();
      }
    }
  } catch (e) {
    // Ignore
  }
}

// ── Dashboard Update ────────────────────────────────────────────────

function updateDashboard(status) {
  // Status card
  if (!isRunning) {
    els.statusCard.className = "status-card status-offline";
    els.statusLabel.textContent = "Offline";
    els.statusDetail.textContent = "Node is not running";
    els.syncSection.style.display = "none";
    els.networkCard.style.display = "none";
    els.statBlocks.textContent = "--";
    els.statDaa.textContent = "--";
    els.statPeers.textContent = "--";
    els.statVersion.textContent = "--";
    return;
  }

  if (!status.rpc_connected) {
    els.statusCard.className = "status-card status-syncing";
    els.statusLabel.textContent = "Starting...";
    els.statusDetail.textContent = "Waiting for kaspad to initialize";
    return;
  }

  if (status.is_synced) {
    els.statusCard.className = "status-card status-synced";
    els.statusLabel.textContent = "Synced";
    els.statusDetail.textContent = "Your node is fully synced with the Kaspa network";
    els.syncSection.style.display = "none";
  } else {
    els.statusCard.className = "status-card status-syncing";
    els.statusLabel.textContent = "Syncing";
    els.statusDetail.textContent = "Downloading and verifying the Kaspa DAG";

    // Show sync progress
    els.syncSection.style.display = "block";
    const percent = Math.round(status.sync_progress * 100);
    els.progressBar.style.width = `${percent}%`;
    els.syncPercent.textContent = `${percent}%`;
    els.syncBlocks.textContent = `${formatNumber(status.block_count)} blocks`;
  }

  // Stats
  els.statBlocks.textContent = formatNumber(status.block_count);
  els.statDaa.textContent = formatNumber(status.virtual_daa_score);
  els.statPeers.textContent = status.peer_count.toString();
  els.statVersion.textContent = status.server_version || "--";

  // Network info
  els.networkCard.style.display = "block";
  els.infoNetwork.textContent = status.network || "--";
  els.infoUtxo.textContent = status.has_utxo_index ? "Enabled" : "Disabled";
}

// ── Update Check ────────────────────────────────────────────────────

async function handleCheckUpdate() {
  els.checkUpdateBtn.disabled = true;
  els.checkUpdateBtn.textContent = "Checking...";
  els.updateStatus.textContent = "";

  try {
    const result = await invoke("check_update");
    if (result.success) {
      if (result.data) {
        els.updateStatus.textContent = `New version available: v${result.data}`;
        els.updateStatus.style.color = "#49EACB";
      } else {
        els.updateStatus.textContent = "You're running the latest version";
        els.updateStatus.style.color = "#8B949E";
      }
    } else {
      els.updateStatus.textContent = `Error: ${result.error}`;
      els.updateStatus.style.color = "#F85149";
    }
  } catch (e) {
    els.updateStatus.textContent = `Error: ${e}`;
  } finally {
    els.checkUpdateBtn.disabled = false;
    els.checkUpdateBtn.textContent = "Check";
  }
}

// ── Utilities ───────────────────────────────────────────────────────

function formatNumber(num) {
  if (num === 0 || num === undefined) return "--";
  return num.toLocaleString("en-US");
}

// ── Boot ────────────────────────────────────────────────────────────

document.addEventListener("DOMContentLoaded", init);
