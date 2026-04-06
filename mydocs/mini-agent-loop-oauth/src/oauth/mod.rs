//! # OAuth 2.0 Module — Authorization Code Flow with PKCE
//!
//! Implements the OAuth 2.0 Authorization Code Grant (RFC 6749 §4.1) with
//! PKCE extension (RFC 7636) for secure tool authentication.
//!
//! ## RFC Compliance
//!
//! | RFC    | Title                                          | Coverage                    |
//! |--------|------------------------------------------------|-----------------------------|
//! | 6749   | OAuth 2.0 Authorization Framework              | Auth code grant, token exchange, refresh |
//! | 6750   | Bearer Token Usage                             | Authorization header format |
//! | 7636   | PKCE                                           | S256 challenge, verifier    |
//! | 8252   | OAuth for Native Apps                          | Loopback redirect           |
//! | 9700   | Security Best Current Practice                 | PKCE required, state param  |
//!
//! ## Architecture (from ironclaw)
//!
//! In ironclaw, OAuth is split across:
//!   - `src/cli/oauth_defaults.rs` — shared OAuth infrastructure
//!   - `src/llm/oauth_helpers.rs` — callback server, landing pages
//!   - `src/tools/mcp/auth.rs` — MCP-specific OAuth 2.1
//!   - `src/extensions/manager.rs` — WASM tool OAuth orchestration
//!
//! We distill all of that into three focused submodules:
//!   - `pkce` — PKCE challenge generation + CSRF state
//!   - `callback` — local loopback callback server
//!   - `token` — token storage, expiry, refresh

pub mod callback;
pub mod pkce;
pub mod token;

use std::collections::HashMap;

pub use callback::callback_url;
pub use pkce::{PkceChallenge, generate_state};
pub use token::{TokenSet, TokenStore};

use token::refresh_access_token;

// ============================================================================
// OAuth Provider Configuration
// ============================================================================

/// Configuration for an OAuth 2.0 provider.
///
/// Maps to ironclaw's tool capabilities.json `auth.oauth` section.
/// Each tool/service declares its OAuth endpoints and requirements.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthProvider {
    /// Human-readable provider name (e.g., "GitHub").
    pub name: String,
    /// Authorization endpoint URL (RFC 6749 §3.1).
    pub authorization_url: String,
    /// Token endpoint URL (RFC 6749 §3.2).
    pub token_url: String,
    /// Client ID (registered with the provider).
    pub client_id: String,
    /// Client secret (None for public clients per RFC 6749 §2.1).
    pub client_secret: Option<String>,
    /// Requested scopes (RFC 6749 §3.3).
    pub scopes: Vec<String>,
    /// Whether to use PKCE (REQUIRED for public clients per RFC 9700).
    pub use_pkce: bool,
    /// Extra parameters for the authorization URL.
    #[serde(default)]
    pub extra_params: HashMap<String, String>,
}

// ============================================================================
// OAuth Flow — Build Authorization URL
// ============================================================================

/// Result of building an OAuth authorization URL.
pub struct AuthUrlResult {
    /// The full authorization URL to open in the browser.
    pub url: String,
    /// PKCE code verifier (must be sent with token exchange).
    pub code_verifier: Option<String>,
    /// CSRF state parameter (must be validated in callback).
    pub state: String,
}

/// Build an OAuth 2.0 authorization URL with PKCE and CSRF state.
///
/// Per RFC 6749 §4.1.1: The authorization request includes:
///   - response_type=code (REQUIRED)
///   - client_id (REQUIRED)
///   - redirect_uri (OPTIONAL but RECOMMENDED)
///   - scope (OPTIONAL)
///   - state (RECOMMENDED, we make it REQUIRED per RFC 9700)
///
/// Per RFC 7636 §4.3: When PKCE is used, also includes:
///   - code_challenge (REQUIRED)
///   - code_challenge_method=S256 (REQUIRED, plain is NOT allowed per RFC 9700)
///
/// Corresponds to: `src/cli/oauth_defaults.rs` → `build_oauth_url()`
pub fn build_auth_url(provider: &OAuthProvider) -> AuthUrlResult {
    let pkce = if provider.use_pkce {
        Some(PkceChallenge::generate())
    } else {
        None
    };

    let state = generate_state();
    let redirect_uri = callback_url();

    // Build the authorization URL (RFC 6749 §4.1.1)
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&state={}",
        provider.authorization_url,
        urlencoding::encode(&provider.client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&state),
    );

    // Add scopes (RFC 6749 §3.3)
    if !provider.scopes.is_empty() {
        url.push_str(&format!(
            "&scope={}",
            urlencoding::encode(&provider.scopes.join(" "))
        ));
    }

    // Add PKCE challenge (RFC 7636 §4.3)
    if let Some(ref pkce) = pkce {
        url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            pkce.challenge
        ));
    }

    // Add extra provider-specific parameters
    for (key, value) in &provider.extra_params {
        url.push_str(&format!(
            "&{}={}",
            urlencoding::encode(key),
            urlencoding::encode(value)
        ));
    }

    AuthUrlResult {
        url,
        code_verifier: pkce.map(|p| p.verifier),
        state,
    }
}

// ============================================================================
// OAuth Flow — Token Exchange
// ============================================================================

/// Response from the OAuth token exchange.
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub scope: Vec<String>,
}

/// Exchange an authorization code for tokens.
///
/// Per RFC 6749 §4.1.3: The token request includes:
///   - grant_type=authorization_code (REQUIRED)
///   - code (REQUIRED)
///   - redirect_uri (REQUIRED if included in auth request)
///   - client_id (REQUIRED for public clients)
///
/// Per RFC 7636 §4.5: When PKCE was used, also includes:
///   - code_verifier (REQUIRED)
///
/// Authentication:
///   - Confidential clients: HTTP Basic auth with client_id:client_secret (RFC 6749 §2.3.1)
///   - Public clients: client_id in request body (RFC 6749 §2.3.1)
///
/// Corresponds to: `src/cli/oauth_defaults.rs` → `exchange_oauth_code()`
pub async fn exchange_code(
    provider: &OAuthProvider,
    code: &str,
    code_verifier: Option<&str>,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let redirect_uri = callback_url();

    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri),
    ];

    // Add PKCE verifier (RFC 7636 §4.5)
    if let Some(verifier) = code_verifier {
        params.push(("code_verifier", verifier.to_string()));
    }

    let mut request = client.post(&provider.token_url);

    if let Some(ref secret) = provider.client_secret {
        // Confidential client: HTTP Basic auth (RFC 6749 §2.3.1)
        request = request.basic_auth(&provider.client_id, Some(secret));
    } else {
        // Public client: client_id in body (RFC 6749 §2.3.1)
        params.push(("client_id", provider.client_id.clone()));
    }

    let response = request
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed ({}): {}", status, body));
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    let access_token = data["access_token"]
        .as_str()
        .ok_or_else(|| {
            let fields: Vec<&str> = data
                .as_object()
                .map(|o| o.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            format!("No 'access_token' in response (fields: {:?})", fields)
        })?
        .to_string();

    let refresh_token = data["refresh_token"].as_str().map(String::from);
    let expires_in = data["expires_in"].as_u64();
    let scope = data["scope"]
        .as_str()
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default();

    Ok(TokenResponse {
        access_token,
        refresh_token,
        expires_in,
        scope,
    })
}

// ============================================================================
// OAuth Flow — Complete Flow Orchestration
// ============================================================================

/// Run the complete OAuth 2.0 Authorization Code flow with PKCE.
///
/// This is the top-level function that orchestrates the entire flow:
///   1. Build authorization URL with PKCE challenge and CSRF state
///   2. Open the URL in the user's browser
///   3. Start local callback server on loopback
///   4. Wait for authorization code callback
///   5. Exchange code for tokens
///   6. Store tokens
///
/// Returns the access token on success.
///
/// Corresponds to the flow in `src/extensions/manager.rs` → WASM tool OAuth
pub async fn run_oauth_flow(
    provider: &OAuthProvider,
    store: &mut TokenStore,
) -> Result<String, String> {
    println!("  🔐 Starting OAuth flow for {}...", provider.name);

    // Step 1: Build authorization URL
    let auth_result = build_auth_url(provider);
    println!("  📋 Authorization URL built (PKCE: {}, state: {}...)",
        auth_result.code_verifier.is_some(),
        &auth_result.state[..8]);

    // Step 2: Bind callback listener BEFORE opening browser
    // (ensures we're ready to receive the callback)
    let listener = callback::bind_callback_listener()
        .await
        .map_err(|e| format!("Failed to start callback listener: {}", e))?;
    println!("  🌐 Callback listener ready on {}", callback_url());

    // Step 3: Open browser
    println!("\n  👉 Opening browser for authorization...");
    println!("     If the browser doesn't open, visit this URL:");
    println!("     {}\n", auth_result.url);

    if let Err(e) = open_browser(&auth_result.url) {
        println!("  ⚠️  Could not open browser: {}", e);
        println!("     Please open the URL above manually.");
    }

    // Step 4: Wait for callback
    println!("  ⏳ Waiting for authorization (5 min timeout)...");
    let code = callback::wait_for_callback(listener, &auth_result.state, &provider.name)
        .await
        .map_err(|e| format!("Callback failed: {}", e))?;
    println!("  ✅ Authorization code received");

    // Step 5: Exchange code for tokens
    println!("  🔄 Exchanging code for tokens...");
    let token_response = exchange_code(provider, &code, auth_result.code_verifier.as_deref())
        .await?;
    println!("  ✅ Tokens received (expires_in: {:?}s)", token_response.expires_in);

    // Step 6: Store tokens
    let token_set = TokenSet::from_response(
        token_response.access_token.clone(),
        token_response.refresh_token,
        token_response.expires_in,
        token_response.scope,
        provider.name.clone(),
    );
    store.store(&provider.name.to_lowercase(), token_set)?;
    println!("  💾 Tokens stored securely");

    Ok(token_response.access_token)
}

/// Get a valid access token for a provider, refreshing if expired.
///
/// Per RFC 9700 §4.13: Clients SHOULD use refresh tokens to obtain
/// new access tokens without user interaction.
#[allow(dead_code)]
pub async fn get_valid_token(
    provider: &OAuthProvider,
    store: &mut TokenStore,
) -> Result<String, String> {
    let provider_key = provider.name.to_lowercase();

    if let Some(token_set) = store.get(&provider_key) {
        if !token_set.is_expired() {
            return Ok(token_set.access_token.clone());
        }

        // Token expired — try refresh
        if let Some(ref refresh_token) = token_set.refresh_token {
            println!("  🔄 Access token expired, refreshing...");
            match refresh_access_token(
                &provider.token_url,
                &provider.client_id,
                provider.client_secret.as_deref(),
                refresh_token,
            )
            .await
            {
                Ok(mut new_tokens) => {
                    new_tokens.provider = provider.name.clone();
                    let access_token = new_tokens.access_token.clone();
                    store.store(&provider_key, new_tokens)?;
                    println!("  ✅ Token refreshed successfully");
                    return Ok(access_token);
                }
                Err(e) => {
                    println!("  ⚠️  Refresh failed: {}", e);
                    println!("  🔐 Starting new OAuth flow...");
                    // Fall through to full OAuth flow
                }
            }
        }
    }

    // No valid token — run full OAuth flow
    run_oauth_flow(provider, store).await
}

/// Attempt to open a URL in the user's default browser.
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ============================================================================
// Built-in Provider Configurations
// ============================================================================

/// GitHub OAuth provider configuration.
///
/// GitHub uses OAuth 2.0 with PKCE support.
/// Register your app at: https://github.com/settings/developers
pub fn github_provider(client_id: &str, client_secret: Option<&str>) -> OAuthProvider {
    OAuthProvider {
        name: "GitHub".to_string(),
        authorization_url: "https://github.com/login/oauth/authorize".to_string(),
        token_url: "https://github.com/login/oauth/access_token".to_string(),
        client_id: client_id.to_string(),
        client_secret: client_secret.map(String::from),
        scopes: vec!["read:user".to_string(), "repo".to_string()],
        use_pkce: true,
        extra_params: HashMap::new(),
    }
}

/// Google OAuth provider configuration.
///
/// Google uses OAuth 2.0 with PKCE (S256).
/// Register your app at: https://console.cloud.google.com/apis/credentials
///
/// Matches ironclaw's built-in Google credentials pattern.
pub fn google_provider(client_id: &str, client_secret: Option<&str>) -> OAuthProvider {
    let mut extra = HashMap::new();
    extra.insert("access_type".to_string(), "offline".to_string());
    extra.insert("prompt".to_string(), "consent".to_string());

    OAuthProvider {
        name: "Google".to_string(),
        authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
        token_url: "https://oauth2.googleapis.com/token".to_string(),
        client_id: client_id.to_string(),
        client_secret: client_secret.map(String::from),
        scopes: vec![
            "https://www.googleapis.com/auth/userinfo.email".to_string(),
            "https://www.googleapis.com/auth/userinfo.profile".to_string(),
        ],
        use_pkce: true,
        extra_params: extra,
    }
}
