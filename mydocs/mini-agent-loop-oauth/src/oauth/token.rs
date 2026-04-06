//! # OAuth 2.0 — Token Management
//!
//! Handles token storage, retrieval, refresh, and expiry tracking.
//!
//! Per RFC 6749 §5.1: Token responses include access_token, token_type,
//! optional expires_in, optional refresh_token, and optional scope.
//!
//! Per RFC 9700 §2.4: Refresh tokens SHOULD be sender-constrained or
//! rotation-based. We implement rotation (new refresh_token on each refresh).
//!
//! Storage is file-based (JSON) for simplicity. In ironclaw, tokens are
//! stored in an encrypted SecretsStore backed by the system keychain.
//!
//! Corresponds to: `src/cli/oauth_defaults.rs` → `store_oauth_tokens()` + refresh logic

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Stored OAuth token set for a single provider.
///
/// Maps to ironclaw's pattern of storing access_token, refresh_token,
/// and scopes as separate secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    /// The access token (Bearer token per RFC 6750).
    pub access_token: String,
    /// The refresh token (RFC 6749 §1.5).
    pub refresh_token: Option<String>,
    /// When the access token expires (computed from expires_in).
    pub expires_at: Option<DateTime<Utc>>,
    /// Scopes granted by the authorization server.
    pub scopes: Vec<String>,
    /// Provider display name (e.g., "GitHub", "Google").
    pub provider: String,
}

impl TokenSet {
    /// Create a new TokenSet from an OAuth token response.
    pub fn from_response(
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        scopes: Vec<String>,
        provider: String,
    ) -> Self {
        let expires_at = expires_in.map(|secs| Utc::now() + Duration::seconds(secs as i64));
        Self {
            access_token,
            refresh_token,
            expires_at,
            scopes,
            provider,
        }
    }

    /// Returns true if the access token has expired or will expire within 60 seconds.
    ///
    /// Per RFC 9700: Clients SHOULD proactively refresh tokens before expiry.
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() + Duration::seconds(60) >= exp,
            None => false, // No expiry info — assume valid
        }
    }
}

/// Simple file-based token store.
///
/// Stores tokens as JSON in `~/.mini-agent/tokens.json`.
/// In production (ironclaw), this would be an encrypted keychain-backed store.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TokenStore {
    /// Map of provider name → token set.
    pub tokens: std::collections::HashMap<String, TokenSet>,
}

impl TokenStore {
    /// Path to the token store file.
    fn store_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".mini-agent").join("tokens.json")
    }

    /// Load the token store from disk.
    pub fn load() -> Self {
        let path = Self::store_path();
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save the token store to disk.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::store_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create token store directory: {}", e))?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize tokens: {}", e))?;
        std::fs::write(&path, content)
            .map_err(|e| format!("Failed to write token store: {}", e))?;

        // Set restrictive permissions (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    /// Store a token set for a provider.
    pub fn store(&mut self, name: &str, token_set: TokenSet) -> Result<(), String> {
        self.tokens.insert(name.to_string(), token_set);
        self.save()
    }

    /// Get a token set for a provider.
    pub fn get(&self, name: &str) -> Option<&TokenSet> {
        self.tokens.get(name)
    }

    /// Remove a token set for a provider.
    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        self.tokens.remove(name);
        self.save()
    }

    /// List all stored providers.
    pub fn list_providers(&self) -> Vec<&str> {
        self.tokens.keys().map(|k| k.as_str()).collect()
    }
}

/// Refresh an expired access token using the refresh token.
///
/// Per RFC 6749 §6: The client sends a POST to the token endpoint with
/// grant_type=refresh_token. The server MAY issue a new refresh token,
/// in which case the client MUST discard the old one.
///
/// Per RFC 9700 §4.13.2: Refresh token rotation — the server issues a
/// new refresh_token with each refresh response.
#[allow(dead_code)]
pub async fn refresh_access_token(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<TokenSet, String> {
    let client = reqwest::Client::new();
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
    ];

    let mut request = client.post(token_url);

    if let Some(secret) = client_secret {
        // Confidential client: use HTTP Basic auth (RFC 6749 §2.3.1)
        request = request.basic_auth(client_id, Some(secret));
    } else {
        // Public client: include client_id in body (RFC 6749 §2.3.1)
        params.push(("client_id", client_id.to_string()));
    }

    let response = request
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Token refresh failed: {} - {}", status, body));
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

    let access_token = data["access_token"]
        .as_str()
        .ok_or("No access_token in refresh response")?
        .to_string();

    let new_refresh = data["refresh_token"].as_str().map(String::from);
    let expires_in = data["expires_in"].as_u64();
    let scope = data["scope"]
        .as_str()
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default();

    Ok(TokenSet::from_response(
        access_token,
        // Use new refresh token if provided (rotation), otherwise keep old one
        new_refresh.or_else(|| Some(refresh_token.to_string())),
        expires_in,
        scope,
        String::new(), // Provider will be set by caller
    ))
}
