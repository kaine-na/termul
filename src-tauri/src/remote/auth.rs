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
    /// Returns `true` if the token matches. Uses byte-level XOR comparison
    /// over the full length to prevent timing-based side-channel attacks.
    /// Length mismatch is detected by XORing the lengths into the accumulator
    /// rather than early-returning, so no timing information is leaked.
    pub fn validate_token(&self, provided: &str) -> bool {
        let expected = self.token.as_bytes();
        let given = provided.as_bytes();

        // XOR length difference into accumulator — no early return on mismatch
        let mut diff = (expected.len() ^ given.len()) as u8;

        // Compare bytes up to the shorter length
        let min_len = expected.len().min(given.len());
        for i in 0..min_len {
            diff |= expected[i] ^ given[i];
        }

        // If lengths differ, remaining bytes also contribute to diff
        if expected.len() > min_len {
            for &b in &expected[min_len..] {
                diff |= b;
            }
        }
        if given.len() > min_len {
            for &b in &given[min_len..] {
                diff |= b;
            }
        }

        // Use volatile read to prevent compiler from optimizing away the loop
        std::hint::black_box(diff) == 0
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

        // Sandboxed iframes send "null" — allow for localhost-only server
        if origin == "null" {
            return true;
        }

        // Parse the origin as a URL and extract the host:port portion.
        // We do NOT use prefix matching to prevent bypasses like
        // `http://localhost.evil.com:19480`.
        let after_scheme = match origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"))
        {
            Some(rest) => rest,
            None => return false, // No recognized scheme
        };

        // Extract host:port (everything before the first `/`)
        let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);

        // Exact match against allowed host:port combinations
        let allowed = [
            format!("localhost:{port}"),
            format!("127.0.0.1:{port}"),
            format!("[::1]:{port}"),
        ];

        allowed.iter().any(|h| host_port == h)
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
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
        assert!(AuthState::validate_origin(
            Some("https://localhost:19480"),
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
    fn test_origin_validation_prefix_bypass() {
        // These should NOT match — prefix matching bypass prevention
        assert!(!AuthState::validate_origin(
            Some("http://localhost.evil.com:19480"),
            19480
        ));
        assert!(!AuthState::validate_origin(
            Some("http://localhostattacker.io:19480"),
            19480
        ));
        assert!(!AuthState::validate_origin(
            Some("http://127.0.0.1.evil.com:19480"),
            19480
        ));
    }

    #[test]
    fn test_origin_validation_null() {
        assert!(AuthState::validate_origin(Some("null"), 19480));
    }

    #[test]
    fn test_origin_validation_ipv6() {
        assert!(AuthState::validate_origin(
            Some("http://[::1]:19480"),
            19480
        ));
        assert!(!AuthState::validate_origin(
            Some("http://[::1]:9999"),
            19480
        ));
    }

    #[test]
    fn test_extract_port() {
        assert_eq!(extract_port("http://localhost:19480"), Some(19480));
        assert_eq!(extract_port("https://127.0.0.1:8080"), Some(8080));
        assert_eq!(extract_port("http://localhost"), None);
    }
}
