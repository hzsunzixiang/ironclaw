//! # OAuth 2.0 — Local Callback Server
//!
//! Implements the local HTTP callback listener for the OAuth 2.0 Authorization
//! Code flow. This is a minimal, ephemeral HTTP server that:
//!
//!   1. Binds to loopback only (127.0.0.1:9876) — per RFC 8252 §7.3
//!   2. Waits for the authorization server to redirect the user's browser
//!   3. Extracts the authorization code from the callback query string
//!   4. Validates the CSRF state parameter
//!   5. Returns a branded HTML page and shuts down
//!
//! Security controls (from ironclaw's `src/llm/oauth_helpers.rs`):
//!   - Loopback-only binding (no remote access)
//!   - 5-minute timeout (prevents dangling listeners)
//!   - CSRF state validation
//!   - HTML escaping for XSS prevention
//!   - Error parameter checking before code extraction
//!
//! Corresponds to: `src/llm/oauth_helpers.rs` → `bind_callback_listener()` + `wait_for_callback()`

use std::collections::HashMap;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Fixed port for the OAuth callback listener.
/// Matches ironclaw's `OAUTH_CALLBACK_PORT` in `src/llm/oauth_helpers.rs`.
pub const CALLBACK_PORT: u16 = 9876;

/// OAuth callback errors.
#[derive(Debug)]
pub enum CallbackError {
    /// The callback port is already in use.
    PortInUse(String),
    /// The user denied authorization at the provider.
    Denied,
    /// Timed out waiting for the callback (5 minutes).
    Timeout,
    /// CSRF state parameter mismatch.
    StateMismatch { expected: String, actual: String },
    /// Generic I/O or network error.
    Io(String),
}

impl std::fmt::Display for CallbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PortInUse(e) => write!(f, "Port {} is in use: {}", CALLBACK_PORT, e),
            Self::Denied => write!(f, "Authorization denied by user"),
            Self::Timeout => write!(f, "Timed out waiting for authorization (5 min)"),
            Self::StateMismatch { expected, actual } => {
                write!(f, "CSRF state mismatch: expected {}, got {}", expected, actual)
            }
            Self::Io(e) => write!(f, "Callback error: {}", e),
        }
    }
}

/// Returns the OAuth callback redirect URI.
///
/// Always uses loopback per RFC 8252 §7.3 (Loopback Interface Redirection).
pub fn callback_url() -> String {
    format!("http://127.0.0.1:{}/callback", CALLBACK_PORT)
}

/// Bind the OAuth callback listener on loopback.
///
/// Per RFC 8252 §7.3: Native apps SHOULD use the loopback interface.
/// Per ironclaw: prefer IPv4 127.0.0.1, fall back to [::1] IPv6.
///
/// Fails fast if the port is already in use (another auth flow running).
pub async fn bind_callback_listener() -> Result<TcpListener, CallbackError> {
    let ipv4_addr = format!("127.0.0.1:{}", CALLBACK_PORT);
    match TcpListener::bind(&ipv4_addr).await {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            Err(CallbackError::PortInUse(e.to_string()))
        }
        Err(_) => {
            // IPv4 not available, fall back to IPv6 loopback
            TcpListener::bind(format!("[::1]:{}", CALLBACK_PORT))
                .await
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AddrInUse {
                        CallbackError::PortInUse(e.to_string())
                    } else {
                        CallbackError::Io(e.to_string())
                    }
                })
        }
    }
}

/// Wait for an OAuth callback and extract the authorization code.
///
/// Listens for a GET /callback?code=...&state=... request, validates the
/// CSRF state parameter, and returns the authorization code.
///
/// Times out after 5 minutes per ironclaw's convention.
pub async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    display_name: &str,
) -> Result<String, CallbackError> {
    let expected_state = expected_state.to_string();
    let display_name = display_name.to_string();

    tokio::time::timeout(Duration::from_secs(300), async move {
        loop {
            let (mut socket, _) = listener
                .accept()
                .await
                .map_err(|e| CallbackError::Io(e.to_string()))?;

            let mut reader = BufReader::new(&mut socket);
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .await
                .map_err(|e| CallbackError::Io(e.to_string()))?;

            // Parse: "GET /callback?code=xxx&state=yyy HTTP/1.1"
            let path = match request_line.split_whitespace().nth(1) {
                Some(p) if p.starts_with("/callback") => p,
                _ => {
                    // Not our callback — return 404
                    let response = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                    continue;
                }
            };

            let query = match path.split('?').nth(1) {
                Some(q) => q,
                None => {
                    let response = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                    continue;
                }
            };

            // Check for error response from authorization server
            // Per RFC 6749 §4.1.2.1: error parameter indicates failure
            if query.contains("error=") {
                let html = landing_html(&display_name, false);
                let response = format!(
                    "HTTP/1.1 400 Bad Request\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\
                     Connection: close\r\n\r\n{}",
                    html
                );
                let _ = socket.write_all(response.as_bytes()).await;
                return Err(CallbackError::Denied);
            }

            // Parse query parameters
            let params: HashMap<&str, String> = query
                .split('&')
                .filter_map(|p| {
                    let mut parts = p.splitn(2, '=');
                    let key = parts.next()?;
                    let val = parts.next().unwrap_or("");
                    Some((
                        key,
                        urlencoding::decode(val)
                            .unwrap_or_else(|_| val.into())
                            .into_owned(),
                    ))
                })
                .collect();

            // Validate CSRF state parameter (RFC 6749 §10.12)
            let actual_state = params.get("state").cloned().unwrap_or_default();
            if actual_state != expected_state {
                let html = landing_html(&display_name, false);
                let response = format!(
                    "HTTP/1.1 403 Forbidden\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\
                     Connection: close\r\n\r\n{}",
                    html
                );
                let _ = socket.write_all(response.as_bytes()).await;
                return Err(CallbackError::StateMismatch {
                    expected: expected_state,
                    actual: actual_state,
                });
            }

            // Extract authorization code
            if let Some(code) = params.get("code") {
                let html = landing_html(&display_name, true);
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\
                     Connection: close\r\n\r\n{}",
                    html
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
                return Ok(code.clone());
            }

            // No code parameter — bad request
            let response = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(response.as_bytes()).await;
        }
    })
    .await
    .map_err(|_| CallbackError::Timeout)?
}

/// Escape a string for safe HTML interpolation (XSS prevention).
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Generate a branded HTML landing page for the OAuth callback result.
/// Matches ironclaw's `landing_html()` in `src/llm/oauth_helpers.rs`.
fn landing_html(provider_name: &str, success: bool) -> String {
    let safe_name = html_escape(provider_name);
    let (icon, heading, subtitle, accent) = if success {
        (
            "✅",
            format!("{} Connected", safe_name),
            "You can close this window and return to your terminal.",
            "#22c55e",
        )
    } else {
        (
            "❌",
            "Authorization Failed".to_string(),
            "The request was denied. You can close this window and try again.",
            "#ef4444",
        )
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Mini Agent - {heading}</title>
<style>
  * {{ margin:0; padding:0; box-sizing:border-box }}
  body {{
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: #0a0a0a; color: #e5e5e5;
    display: flex; justify-content: center; align-items: center; min-height: 100vh;
  }}
  .card {{
    text-align: center; padding: 48px 40px; max-width: 420px;
    border: 1px solid #262626; border-radius: 16px; background: #141414;
  }}
  .icon {{ font-size: 48px; margin-bottom: 16px; }}
  h1 {{ font-size: 22px; font-weight: 600; margin-bottom: 8px; color: #fafafa; }}
  p {{ font-size: 14px; color: #a3a3a3; line-height: 1.5; }}
  .accent {{ color: {accent}; }}
</style>
</head>
<body>
  <div class="card">
    <div class="icon">{icon}</div>
    <h1>{heading}</h1>
    <p>{subtitle}</p>
  </div>
</body>
</html>"#,
        heading = heading,
        icon = icon,
        subtitle = subtitle,
        accent = accent,
    )
}
