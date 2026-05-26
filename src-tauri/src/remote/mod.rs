//! Remote Terminal module
//!
//! Provides HTTP + WebSocket server for accessing Termul terminals from a web browser.
//! Architecture: Axum server with token-based authentication and per-terminal broadcast channels.
//!
//! # Security
//! - Binds to `127.0.0.1` by default (LAN-only with explicit opt-in)
//! - Token-based authentication on WebSocket upgrade
//! - Origin validation to prevent Cross-Site WebSocket Hijacking (CSWSH)

pub mod auth;
pub mod server;
pub mod ws;

/// Embedded web client HTML (served at `GET /`)
pub static INDEX_HTML: &str = include_str!("static/index.html");

/// Default port for the remote terminal server
pub const DEFAULT_PORT: u16 = 19480;

/// Maximum number of concurrent WebSocket connections per terminal
pub const MAX_CONNECTIONS_PER_TERMINAL: usize = 10;

/// Global maximum WebSocket connections
pub const MAX_TOTAL_CONNECTIONS: usize = 50;

/// Broadcast channel capacity per terminal
pub const BROADCAST_CAPACITY: usize = 128;
