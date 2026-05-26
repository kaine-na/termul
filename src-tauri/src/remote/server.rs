//! Axum HTTP + WebSocket server for remote terminal access.
//!
//! # Architecture
//! ```text
//! Browser → HTTP GET /           → Serve embedded index.html
//! Browser → HTTP GET /api/terms  → List active terminals (JSON)
//! Browser → WS GET  /ws?token=X  → Upgrade to WebSocket terminal session
//! ```
//!
//! The server runs in a background tokio task and can be started/stopped
//! via Tauri commands from the desktop frontend.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, Method, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower_http::cors::{Any, CorsLayer};

use crate::pty::PtyManager;

use super::auth::{AuthState, WsAuthQuery};
use super::ws;
use super::{BROADCAST_CAPACITY, MAX_TOTAL_CONNECTIONS};

/// Shared application state passed to all Axum handlers.
#[derive(Clone)]
pub struct AppState {
    /// Reference to the existing PTY manager (owns all terminals)
    pub pty_manager: Arc<PtyManager>,
    /// Authentication state (token generation + validation)
    pub auth: Arc<AuthState>,
    /// Global active WebSocket connection counter
    pub connection_count: Arc<AtomicUsize>,
    /// Server port (needed for origin validation)
    pub port: u16,
}

/// Handle for controlling a running remote server.
///
/// Returned by `start_server()` and used to stop the server later.
pub struct ServerHandle {
    /// Sends shutdown signal to the server
    shutdown_tx: oneshot::Sender<()>,
    /// Background task join handle
    task_handle: tokio::task::JoinHandle<()>,
    /// Port the server is listening on
    port: u16,
    /// Authentication token
    token: String,
}

impl ServerHandle {
    /// Get the port the server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the authentication token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Stop the remote server gracefully.
    ///
    /// Sends a shutdown signal and waits for the server task to complete.
    pub async fn stop(self) -> Result<(), String> {
        let _ = self.shutdown_tx.send(());
        self.task_handle
            .await
            .map_err(|e| format!("Server task panicked: {e}"))
    }
}

/// Start the remote terminal HTTP + WebSocket server.
///
/// Binds to `127.0.0.1:{port}` (or `0.0.0.0:{port}` if `bind_lan` is true),
/// spawns the Axum server in a background tokio task, and returns a `ServerHandle`
/// for controlling the server lifecycle.
///
/// # Arguments
/// * `pty_manager` - Shared PTY manager for terminal access
/// * `port` - TCP port to listen on
/// * `bind_lan` - If true, bind to `0.0.0.0` (all interfaces) instead of `127.0.0.1`
pub async fn start_server(
    pty_manager: Arc<PtyManager>,
    port: u16,
    bind_lan: bool,
) -> Result<ServerHandle, String> {
    let auth = AuthState::new();
    let token = auth.token().to_string();

    let bind_addr: SocketAddr = if bind_lan {
        format!("0.0.0.0:{port}")
    } else {
        format!("127.0.0.1:{port}")
    }
    .parse()
    .map_err(|e| format!("Invalid bind address: {e}"))?;

    let state = AppState {
        pty_manager,
        auth: Arc::new(auth),
        connection_count: Arc::new(AtomicUsize::new(0)),
        port,
    };

    let app = build_router(state);

    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|e| format!("Failed to bind to {bind_addr}: {e}"))?;

    log::info!(
        "[RemoteServer] Listening on {} (LAN: {})",
        bind_addr,
        bind_lan
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let task_handle = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
                log::info!("[RemoteServer] Shutdown signal received");
            })
            .await
            .unwrap_or_else(|e| {
                log::error!("[RemoteServer] Server error: {e}");
            });
    });

    Ok(ServerHandle {
        shutdown_tx,
        task_handle,
        port,
        token,
    })
}

/// Build the Axum router with all routes and middleware.
fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_private_network(true);

    Router::new()
        .route("/", get(serve_index))
        .route("/api/terminals", get(list_terminals))
        .route("/ws", get(ws_upgrade))
        .route("/health", get(health_check))
        .layer(cors)
        .with_state(state)
}

/// `GET /` — Serve the embedded web client HTML.
async fn serve_index() -> Html<&'static str> {
    Html(super::INDEX_HTML)
}

/// `GET /health` — Health check endpoint.
async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "termul-remote"
    }))
}

/// `GET /api/terminals` — List all active terminals.
///
/// Returns a JSON array of terminal info objects.
/// Requires `?token=<TOKEN>` query parameter.
async fn list_terminals(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<WsAuthQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Validate token
    let token = query.token.as_deref().unwrap_or("");
    if !state.auth.validate_token(token) {
        log::warn!("[RemoteServer] Unauthorized terminal list request");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let terminals = state.pty_manager.list_active();
    let count = terminals.len();

    Ok(Json(serde_json::json!({
        "terminals": terminals,
        "count": count,
        "maxConnections": MAX_TOTAL_CONNECTIONS,
        "broadcastCapacity": BROADCAST_CAPACITY,
    })))
}

/// `GET /ws?token=<TOKEN>` — WebSocket upgrade handler.
///
/// Validates authentication and origin before upgrading the connection.
/// On success, hands off to `ws::handle_terminal_ws` for the actual
/// terminal I/O relay.
async fn ws_upgrade(
    ws: axum::extract::WebSocketUpgrade,
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<WsAuthQuery>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    // 1. Validate token
    let token = query.token.as_deref().unwrap_or("");
    if !state.auth.validate_token(token) {
        log::warn!("[RemoteServer] Unauthorized WebSocket connection attempt");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // 2. Validate Origin header (CSWSH protection)
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if !AuthState::validate_origin(origin, state.port) {
        log::warn!(
            "[RemoteServer] Rejected WebSocket from suspicious origin: {:?}",
            origin
        );
        return Err(StatusCode::FORBIDDEN);
    }

    // 3. Check connection limit
    let current = state.connection_count.load(Ordering::SeqCst);
    if current >= MAX_TOTAL_CONNECTIONS {
        log::warn!(
            "[RemoteServer] Connection limit reached ({}/{})",
            current,
            MAX_TOTAL_CONNECTIONS
        );
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // 4. Upgrade to WebSocket
    log::info!("[RemoteServer] Upgrading WebSocket connection (origin: {:?})", origin);

    Ok(ws.on_upgrade(move |socket| ws::handle_terminal_ws(socket, state)))
}
