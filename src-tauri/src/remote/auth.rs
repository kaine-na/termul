//! Authentication module for the remote terminal server.
//!
//! Provides token-based authentication and origin validation to prevent
//! Cross-Site WebSocket Hijacking (CSWSH) attacks.
//!
//! # Security Model
//! - A random UUID v4 token is generated on each server start
//! - Token must be passed as `?token=<TOKEN>` query parameter on WebSocket upgrade
//! - Origin header is validated against known safe patterns
//! - Token is single-use per session (regenerated on server restart)

use uuid::Uuid;

/// Authentication state for the remote terminal server.
#[derive(Debug, Clone)]
pub struct AuthState {
    /// The authentication token (UUID v4)
    token: String,
}

impl AuthState {
    /// Create a new `AuthState` with a freshly generated token.
    pub fn new() -> Self {
        let token = Uuid::new_v4().to_string();
        log::info!("[RemoteAuth] Generated new session token");
        Self { token }
    }

    /// Get the current authentication token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Validate a provided token using constant-time comparison.
    ///
    /// Returns `true` if the token matches. Uses constant-time comparison
    /// to prevent timing-based side-channel attacks.
    pub fn validate_token(&self, provided: &str) -> bool {
        // Constant-time comparison to prevent timing attacks
        if provided.len() != self.token.len() {
            return false;
        }
        let mut result = 0u8;
        for (a, b) in self.token.bytes().zip(provided.bytes()) {
            result |= a ^ b;
        }
        result == 0
    }

    /// Validate the `Origin` header from a WebSocket upgrade request.
    ///
    /// Allows:
    /// - No origin header (direct connections, curl, etc.)
    /// - `localhost` origins (local development)
    /// - `127.0.0.1` origins (local connections)
    /// - Origins matching the server's own address
    ///
    /// Rejects:
    /// - Origins from external domains (potential CSWSH)
    pub fn validate_origin(origin: Option<&str>, port: u16) -> bool {
        let Some(origin) = origin else {
            // No origin header — allow (direct connections, non-browser clients)
            return true;
        };

        let allowed_prefixes = [
            "http://localhost",
            "https://localhost",
            "http://127.0.0.1",
            "https://127.0.0.1",
            "http://[::1]",
            "https://[::1]",
            "null", // file:// or sandboxed origins
        ];

        // Check if origin matches localhost patterns
        for prefix in &allowed_prefixes {
            if origin.starts_with(prefix) {
                // Verify port matches if specified in origin
                if let Some(origin_port) = extract_port(origin) {
                    return origin_port == port;
                }
                return true;
            }
        }

        // Also allow the exact server address
        let self_addresses = [
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ];
        self_addresses.iter().any(|addr| origin == addr)
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract port number from an origin URL.
fn extract_port(origin: &str) -> Option<u16> {
    // Handle origins like "http://localhost:19480" or "http://127.0.0.1:19480"
    let after_scheme = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))?;

    // Find the port after the last colon
    let colon_pos = after_scheme.rfind(':')?;
    let port_str = &after_scheme[colon_pos + 1..];

    // Port might be followed by a path
    let port_str = port_str.split('/').next()?;
    port_str.parse().ok()
}

/// Query parameters for WebSocket authentication.
#[derive(Debug, serde::Deserialize)]
pub struct WsAuthQuery {
    pub token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation() {
        let auth = AuthState::new();
        assert!(!auth.token().is_empty());
        assert_eq!(auth.token().len(), 36); // UUID v4 format
    }

    #[test]
    fn test_token_validation_correct() {
        let auth = AuthState::new();
        let token = auth.token().to_string();
        assert!(auth.validate_token(&token));
    }

    #[test]
    fn test_token_validation_wrong() {
        let auth = AuthState::new();
        assert!(!auth.validate_token("wrong-token"));
        assert!(!auth.validate_token(""));
    }

    #[test]
    fn test_token_validation_different_length() {
        let auth = AuthState::new();
        assert!(!auth.validate_token("short"));
    }

    #[test]
    fn test_origin_validation_no_origin() {
        assert!(AuthState::validate_origin(None, 19480));
    }

    #[test]
    fn test_origin_validation_localhost() {
        assert!(AuthState::validate_origin(
            Some("http://localhost:19480"),
            19480
        ));
        assert!(AuthState::validate_origin(
            Some("http://127.0.0.1:19480"),
            19480
        ));
    }

    #[test]
    fn test_origin_validation_wrong_port() {
        assert!(!AuthState::validate_origin(
            Some("http://localhost:9999"),
            19480
        ));
    }

    #[test]
    fn test_origin_validation_external() {
        assert!(!AuthState::validate_origin(
            Some("http://evil.com"),
            19480
        ));
        assert!(!AuthState::validate_origin(
            Some("https://attacker.io:19480"),
            19480
        ));
    }

    #[test]
    fn test_origin_validation_null() {
        assert!(AuthState::validate_origin(Some("null"), 19480));
    }

    #[test]
    fn test_extract_port() {
        assert_eq!(extract_port("http://localhost:19480"), Some(19480));
        assert_eq!(extract_port("https://127.0.0.1:8080"), Some(8080));
        assert_eq!(extract_port("http://localhost"), None);
    }
}
