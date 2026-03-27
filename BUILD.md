# MyKAI Node — Build Instructions

## Prerequisites

1. **Rust** (1.77+): https://rustup.rs/
2. **Node.js** (18+): https://nodejs.org/
3. **Tauri CLI**: `cargo install tauri-cli --version "^2"`

On Windows you also need:
- Visual Studio Build Tools (C++ workload)
- WebView2 (pre-installed on Windows 10 21H2+)

## Development

```bash
cd mykai-node
npm install
cargo tauri dev
```

This opens the app in dev mode with hot-reload for the frontend.

## Build for Production

```bash
cargo tauri build
```

This produces:
- `src-tauri/target/release/mykai-node.exe` — standalone binary
- `src-tauri/target/release/bundle/nsis/` — NSIS installer (.exe)

The NSIS installer is the single file you distribute to users.

## Architecture

```
User clicks installer (.exe)
    → Installs MyKAI Node app
    → On first launch: downloads kaspad from GitHub
    → Starts kaspad in private mode (--maxinpeers=0)
    → Dashboard shows sync status, block height, peers
    → Close button minimizes to system tray (node keeps running)
    → Quit from tray menu stops kaspad and exits
```

### Rust Backend (src-tauri/src/)
- `main.rs` — App entry, Tauri setup, system tray, window management
- `config.rs` — Persistent configuration (JSON file in %LOCALAPPDATA%)
- `kaspad_manager.rs` — Download, start, stop, update kaspad binary
- `rpc_client.rs` — Monitor kaspad via wRPC-JSON (WebSocket on port 18110)
- `commands.rs` — Tauri commands bridging frontend ↔ backend
- `autostart.rs` — Windows registry auto-start management

### Frontend (src/)
- `index.html` — App structure (setup wizard + dashboard)
- `styles.css` — Dark theme with Kaspa accent (#49EACB)
- `app.js` — Frontend logic, polls backend every 3 seconds

### Key kaspad Configuration
- Binary source: `kaspanet/rusty-kaspa` GitHub releases
- Private mode: `--maxinpeers=0` (no inbound connections)
- UTXO index: `--utxoindex` (enabled for wallet compatibility)
- RPC: `--rpclisten-json=127.0.0.1:18110` (localhost only)
- Data: `%LOCALAPPDATA%\MyKAI Node\data\`

## Security Notes
- RPC bound to localhost only (127.0.0.1)
- Zip extraction validates against path traversal
- No secrets stored — kaspad runs with default configuration
- Auto-start uses standard Windows registry (HKCU\...\Run)
