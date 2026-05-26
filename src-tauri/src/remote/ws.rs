//! WebSocket handler for remote terminal I/O.
//!
//! Handles the bidirectional relay between a web browser (xterm.js) and a
//! Termul PTY session.
//!
//! # Protocol
//!
//! ## Client → Server (JSON text frames)
//! ```json
//! { "type": "input", "data": "ls\n" }
//! { "type": "resize", "cols": 120, "rows": 40 }
//! { "type": "attach", "terminal_id": "terminal-123" }
//! ```
//!
//! ## Server → Client (JSON text frames)
//! ```json
//! { "type": "data", "data": "..." }
//! { "type": "terminal_list", "terminals": [...] }
//! { "type": "connected", "terminalId": "terminal-123" }
//! { "type": "exit", "terminalId": "terminal-123" }
//! { "type": "error", "message": "Terminal not found" }
//! ```

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::time;

use super::server::AppState;
use super::MAX_CONNECTIONS_PER_TERMINAL;

/// Message from client to server.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    /// User input (keystrokes) to send to the PTY
    #[serde(rename = "input")]
    Input { data: String },

    /// Terminal resize request
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },

    /// Attach to a specific terminal
    #[serde(rename = "attach")]
    Attach { terminal_id: String },
}

/// Message from server to client.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    /// Terminal output data
    #[serde(rename = "data")]
    Data { data: String },

    /// List of active terminals
    #[serde(rename = "terminal_list")]
    TerminalList {
        terminals: Vec<crate::pty::TerminalInfo>,
    },

    /// Successfully connected to a terminal
    #[serde(rename = "connected")]
    Connected {
        #[serde(rename = "terminalId")]
        terminal_id: String,
    },

    /// Terminal process exited
    #[serde(rename = "exit")]
    Exit {
        #[serde(rename = "terminalId")]
        terminal_id: String,
    },

    /// Error message
    #[serde(rename = "error")]
    Error { message: String },
}

impl ServerMessage {
    /// Serialize to JSON string for WebSocket text frame.
    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ServerMessage serialization should never fail")
    }
}

/// WebSocket handler for terminal I/O relay.
///
/// Lifecycle:
/// 1. Send terminal list to client
/// 2. Wait for `attach` message from client (with timeout)
/// 3. Subscribe to terminal's broadcast channel
/// 4. Bidirectional relay: PTY output → WS, WS input → PTY
/// 5. Cleanup on disconnect
pub async fn handle_terminal_ws(socket: WebSocket, state: AppState) {
    // Increment global connection count (atomically, with limit check)
    let prev = state.connection_count.fetch_add(1, Ordering::SeqCst);
    if prev >= super::MAX_TOTAL_CONNECTIONS {
        // Over limit — rollback and close
        state.connection_count.fetch_sub(1, Ordering::SeqCst);
        log::warn!("[RemoteWS] Connection rejected: limit reached ({prev})");
        return;
    }
    log::info!("[RemoteWS] New WebSocket connection ({}/{})", prev + 1, super::MAX_TOTAL_CONNECTIONS);

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Send initial terminal list
    let terminals = state.pty_manager.list_active();
    let list_msg = ServerMessage::TerminalList { terminals };
    if ws_tx
        .send(Message::Text(list_msg.to_json()))
        .await
        .is_err()
    {
        state.connection_count.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    // Wait for attach message (with timeout to prevent resource exhaustion)
    const ATTACH_TIMEOUT: Duration = Duration::from_secs(30);
    let terminal_id = match time::timeout(ATTACH_TIMEOUT, async {
        loop {
            match ws_rx.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(ClientMessage::Attach { terminal_id }) =
                        serde_json::from_str::<ClientMessage>(&text)
                    {
                        return Some(terminal_id);
                    }
                    let err_msg = ServerMessage::Error {
                        message: "Expected attach message".to_string(),
                    };
                    let _ = ws_tx.send(Message::Text(err_msg.to_json())).await;
                }
                Some(Ok(Message::Close(_))) | None => return None,
                _ => continue,
            }
        }
    })
    .await
    {
        Ok(Some(id)) => id,
        Ok(None) => {
            log::info!("[RemoteWS] Client disconnected before attaching");
            state.connection_count.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        Err(_) => {
            log::warn!("[RemoteWS] Attach timeout ({}s)", ATTACH_TIMEOUT.as_secs());
            let err_msg = ServerMessage::Error {
                message: "Attach timeout".to_string(),
            };
            let _ = ws_tx.send(Message::Text(err_msg.to_json())).await;
            state.connection_count.fetch_sub(1, Ordering::SeqCst);
            return;
        }
    };

    // Atomically check and reserve per-terminal connection slot (TOCTOU-safe)
    let terminal_instance = match state.pty_manager.get(&terminal_id) {
        Some(inst) => inst,
        None => {
            let err_msg = ServerMessage::Error {
                message: format!("Terminal not found: {terminal_id}"),
            };
            let _ = ws_tx.send(Message::Text(err_msg.to_json())).await;
            state.connection_count.fetch_sub(1, Ordering::SeqCst);
            return;
        }
    };

    if !terminal_instance.try_add_remote_connection(MAX_CONNECTIONS_PER_TERMINAL) {
        let err_msg = ServerMessage::Error {
            message: format!(
                "Connection limit per terminal reached ({MAX_CONNECTIONS_PER_TERMINAL})"
            ),
        };
        let _ = ws_tx.send(Message::Text(err_msg.to_json())).await;
        state.connection_count.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    // Subscribe to broadcast channel
    let mut broadcast_rx = match state.pty_manager.subscribe_broadcast(&terminal_id) {
        Some(rx) => rx,
        None => {
            let err_msg = ServerMessage::Error {
                message: format!("Terminal not found: {terminal_id}"),
            };
            let _ = ws_tx.send(Message::Text(err_msg.to_json())).await;
            state.connection_count.fetch_sub(1, Ordering::SeqCst);
            return;
        }
    };

    // Add renderer ref to prevent orphan cleanup
    let renderer_id = format!("remote-ws-{}", uuid::Uuid::new_v4());
    let _ = state
        .pty_manager
        .add_renderer_ref(&terminal_id, &renderer_id);

    // Send connected confirmation
    let connected_msg = ServerMessage::Connected {
        terminal_id: terminal_id.clone(),
    };
    let _ = ws_tx
        .send(Message::Text(connected_msg.to_json()))
        .await;
    log::info!("[RemoteWS] Attached to terminal: {terminal_id}");

    // Bidirectional relay loop
    let mut ping_interval = time::interval(Duration::from_secs(30));
    let mut lag_count = 0u64;

    loop {
        tokio::select! {
            result = broadcast_rx.recv() => {
                match result {
                    Ok(data) => {
                        let text = String::from_utf8_lossy(&data).to_string();
                        let msg = ServerMessage::Data { data: text };
                        if ws_tx.send(Message::Text(msg.to_json())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        lag_count += n;
                        log::warn!("[RemoteWS] Broadcast lagged: +{n} (total: {lag_count})");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        let exit_msg = ServerMessage::Exit {
                            terminal_id: terminal_id.clone(),
                        };
                        let _ = ws_tx.send(Message::Text(exit_msg.to_json())).await;
                        break;
                    }
                }
            }

            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_client_input(&text, &state, &terminal_id, &mut ws_tx).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        log::warn!("[RemoteWS] WS recv error: {e}");
                        break;
                    }
                    _ => {} // Ignore binary/ping/pong
                }
            }

            _ = ping_interval.tick() => {
                if ws_tx.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        }
    }

    // Cleanup
    let _ = state
        .pty_manager
        .remove_renderer_ref(&terminal_id, &renderer_id);
    // Release atomic remote connection slot
    if let Some(inst) = state.pty_manager.get(&terminal_id) {
        inst.remove_remote_connection();
    }
    state.connection_count.fetch_sub(1, Ordering::SeqCst);
    log::info!("[RemoteWS] Connection closed: {terminal_id}");
}

/// Handle a single client input message (input, resize).
async fn handle_client_input(
    text: &str,
    state: &AppState,
    terminal_id: &str,
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) {
    match serde_json::from_str::<ClientMessage>(text) {
        Ok(ClientMessage::Input { data }) => {
            if let Err(e) = state.pty_manager.write(terminal_id, &data).await {
                log::warn!("[RemoteWS] Write error: {e}");
            }
        }
        Ok(ClientMessage::Resize { cols, rows }) => {
            // Note: resize is DISABLED for web clients because the PTY is shared
            // with the desktop app. Resizing from web would affect the desktop user's
            // terminal dimensions. This may be re-enabled when per-client virtual
            // terminals are implemented.
            log::debug!(
                "[RemoteWS] Resize request ({cols}x{rows}) ignored — shared PTY"
            );
        }
        Ok(ClientMessage::Attach { .. }) => {
            let err_msg = ServerMessage::Error {
                message: "Re-attach not supported. Reconnect.".to_string(),
            };
            let _ = ws_tx.send(Message::Text(err_msg.to_json())).await;
        }
        Err(e) => {
            log::warn!("[RemoteWS] Invalid message: {e}");
        }
    }
}
