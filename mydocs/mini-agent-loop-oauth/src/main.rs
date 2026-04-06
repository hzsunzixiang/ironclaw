//! # Mini Agent Loop — IronClaw Core Distilled + OAuth 2.0
//!
//! This is a minimal, standalone extraction of IronClaw's agentic loop
//! with OAuth 2.0 Authorization Code + PKCE support for tool authentication.
//!
//! ```text
//! stdin → Agentic Loop → LLM → Tool Execute → Response → stdout
//!                                    ↕
//!                              OAuth 2.0 Flow
//!                    (PKCE + loopback callback + token refresh)
//! ```
//!
//! ## Module Structure:
//!
//! | Module              | IronClaw source                          | Purpose                        |
//! |---------------------|------------------------------------------|--------------------------------|
//! | `llm::provider`     | `src/llm/provider.rs`                    | LLM abstraction trait          |
//! | `llm` (types)       | `src/llm/provider.rs`                    | ChatMessage, ToolCall, etc.    |
//! | `llm::openai`       | (replaces real provider configs)         | OpenAI-compatible provider     |
//! | `tools`             | `src/tools/tool.rs` + `execute.rs`       | Tool trait & execution         |
//! | `tools::calculator`  | (like any builtin tool)                  | Example tool                   |
//! | `agent`             | `src/agent/agentic_loop.rs`              | The core loop engine           |
//! | `oauth`             | `src/cli/oauth_defaults.rs` + helpers    | OAuth 2.0 + PKCE               |
//! | `oauth::pkce`       | PKCE challenge generation                | RFC 7636                       |
//! | `oauth::callback`   | Local loopback callback server           | RFC 8252                       |
//! | `oauth::token`      | Token storage, expiry, refresh           | RFC 6749 §5-6                  |
//!
//! ## How to run:
//!
//! ```bash
//! cargo run
//! ```

mod agent;
mod llm;
mod oauth;
mod tools;

use std::io::{self, Write};

use agent::{run_agentic_loop, AgenticLoopConfig, LoopOutcome};
use llm::{ChatMessage, HaiConfig, LlmProvider, OpenAiCompatibleProvider};
use oauth::{OAuthProvider, TokenStore};
use tools::{CalculatorTool, ToolRegistry};

// ============================================================================
// Main — The Entry Point (replaces Channel + Agent::run())
// ============================================================================
// In IronClaw, the flow is:
//   Channel::start() → MessageStream → Agent::run() → handle_message()
//     → SubmissionParser::parse() → process_user_input()
//       → get_or_create_session() → start_turn() → run_agentic_loop()
//         → complete_turn() → channel.respond()
//
// We replace ALL of that with a simple stdin loop:
//   stdin → build messages → run_agentic_loop() → print response
//
// OAuth integration adds:
//   /auth <provider> → run OAuth 2.0 flow → store tokens
//   /tokens → list stored tokens
//   /revoke <provider> → remove stored tokens
// ============================================================================

#[tokio::main]
async fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║       Mini Agent Loop — IronClaw Core + OAuth 2.0          ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║  This demo shows the core agentic loop + OAuth 2.0:        ║");
    println!("║    stdin → LLM → Tool Execute → Response → stdout          ║");
    println!("║    + OAuth 2.0 Authorization Code with PKCE                ║");
    println!("║                                                            ║");
    println!("║  Config: ./HAI_WOA.json (real LLM API)                     ║");
    println!("║                                                            ║");
    println!("║  Try:                                                      ║");
    println!("║    • \"What is 42 + 58?\"                                    ║");
    println!("║    • \"Calculate 100 divided by 3\"                          ║");
    println!("║    • \"Hello\" (direct response, no tool)                    ║");
    println!("║    • \"/model <name>\" to switch model                       ║");
    println!("║    • \"/models\" to list available models                    ║");
    println!("║                                                            ║");
    println!("║  OAuth Commands:                                           ║");
    println!("║    • \"/auth github <client_id> [secret]\" — OAuth login     ║");
    println!("║    • \"/auth google <client_id> [secret]\" — OAuth login     ║");
    println!("║    • \"/auth custom <json_file>\" — custom provider          ║");
    println!("║    • \"/tokens\" — list stored OAuth tokens                  ║");
    println!("║    • \"/revoke <provider>\" — remove stored tokens           ║");
    println!("║    • \"quit\" to exit                                        ║");
    println!("║                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // ── Setup (corresponds to Agent::new() in IronClaw) ──

    // 1. Create LLM provider from ./HAI_WOA.json config
    let hai_config = HaiConfig::load().unwrap_or_else(|e| {
        eprintln!("❌ Failed to load ./HAI_WOA.json: {}", e);
        eprintln!("   Please copy HAI_WOA.template.json to HAI_WOA.json and fill in your API key.");
        std::process::exit(1);
    });

    println!("✅ Loaded config from ./HAI_WOA.json");
    println!("   Model: {}", hai_config.model);
    println!("   Base URL: {}", hai_config.base_url().unwrap_or_default());
    println!(
        "   Available shortcuts: {}",
        hai_config
            .list_models()
            .iter()
            .map(|(k, v)| format!("{} → {}", k, v))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut current_model = hai_config.resolve_model(None);

    let make_provider = |model: &str, config: &HaiConfig| -> Result<Box<dyn LlmProvider>, String> {
        Ok(Box::new(OpenAiCompatibleProvider::new(
            config.api_key()?,
            config.base_url()?,
            model.to_string(),
        )))
    };

    let mut llm: Box<dyn LlmProvider> =
        make_provider(&current_model, &hai_config).unwrap_or_else(|e| {
            eprintln!("❌ Failed to create LLM provider: {}", e);
            std::process::exit(1);
        });

    println!();

    // 2. Create tool registry and register tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CalculatorTool));

    // 3. Load OAuth token store
    let mut token_store = TokenStore::load();
    let stored_providers = token_store.list_providers();
    if !stored_providers.is_empty() {
        println!("🔐 OAuth tokens loaded for: {}", stored_providers.join(", "));
    }

    // 4. Helper to build system prompt (model-agnostic)
    let make_system_prompt = || -> ChatMessage {
        ChatMessage::system(
            "You are a helpful assistant. \
             You have access to a calculator tool. \
             When the user asks a math question, use the calculator tool to compute the answer. \
             For non-math questions, respond directly. \
             When asked about your identity, respond based on your own knowledge."
                .to_string(),
        )
    };

    // 5. Loop config
    let config = AgenticLoopConfig::default();

    // ── Main Loop (corresponds to Agent::run() event loop) ──

    loop {
        // ── Read input (replaces Channel::start() → MessageStream) ──
        print!("\n🧑 You: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => break, // EOF reached
            Err(_) => break,
            _ => {}
        }
        let input = input.trim();

        if input.is_empty() {
            continue;
        }
        if input == "quit" || input == "exit" {
            println!("👋 Goodbye!");
            break;
        }

        // ── Handle slash commands ──
        if input == "/models" {
            println!("\n📋 Available models:");
            println!("   (default) → {}", hai_config.model);
            for (shortcut, model_id) in hai_config.list_models() {
                let marker = if current_model == model_id {
                    " ← current"
                } else {
                    ""
                };
                println!("   {} → {}{}", shortcut, model_id, marker);
            }
            println!("   Current: {}", current_model);
            continue;
        }
        if input.starts_with("/model ") {
            let model_name = input.strip_prefix("/model ").unwrap().trim();
            let resolved = hai_config.resolve_model(Some(model_name));
            current_model = resolved.clone();
            match make_provider(&resolved, &hai_config) {
                Ok(p) => {
                    llm = p;
                    println!("✅ Switched to model: {}", resolved);
                }
                Err(e) => println!("❌ Failed to switch model: {}", e),
            }
            continue;
        }

        // ── OAuth commands ──
        if input.starts_with("/auth ") {
            handle_auth_command(input, &mut token_store).await;
            continue;
        }
        if input == "/tokens" {
            handle_tokens_command(&token_store);
            continue;
        }
        if input.starts_with("/revoke ") {
            handle_revoke_command(input, &mut token_store);
            continue;
        }

        // ── Build conversation context ──
        let mut messages = vec![make_system_prompt(), ChatMessage::user(input)];

        // ── Run the agentic loop ──
        println!("\n🤖 Agent thinking...");
        match run_agentic_loop(llm.as_ref(), &registry, &mut messages, &config).await {
            Ok(LoopOutcome::Response(text)) => {
                println!("\n🤖 Agent: {}", text);
            }
            Ok(LoopOutcome::MaxIterations) => {
                println!("\n⚠️  Agent: Reached maximum iterations without a final response.");
            }
            Err(e) => {
                println!("\n❌ Error: {}", e);
            }
        }
    }
}

// ============================================================================
// OAuth Command Handlers
// ============================================================================

/// Handle `/auth <provider> <client_id> [client_secret]` command.
///
/// Supports built-in providers (github, google) and custom providers via JSON file.
async fn handle_auth_command(input: &str, store: &mut TokenStore) {
    let parts: Vec<&str> = input.strip_prefix("/auth ").unwrap().split_whitespace().collect();

    if parts.is_empty() {
        println!("\n📖 Usage:");
        println!("   /auth github <client_id> [client_secret]");
        println!("   /auth google <client_id> [client_secret]");
        println!("   /auth custom <json_file>");
        println!("\n   Register OAuth apps at:");
        println!("   • GitHub: https://github.com/settings/developers");
        println!("   • Google: https://console.cloud.google.com/apis/credentials");
        println!("   Redirect URI: {}", oauth::callback_url());
        return;
    }

    let provider_name = parts[0].to_lowercase();
    let provider = match provider_name.as_str() {
        "github" => {
            if parts.len() < 2 {
                println!("❌ Usage: /auth github <client_id> [client_secret]");
                return;
            }
            let secret = parts.get(2).map(|s| *s);
            oauth::github_provider(parts[1], secret)
        }
        "google" => {
            if parts.len() < 2 {
                println!("❌ Usage: /auth google <client_id> [client_secret]");
                return;
            }
            let secret = parts.get(2).map(|s| *s);
            oauth::google_provider(parts[1], secret)
        }
        "custom" => {
            if parts.len() < 2 {
                println!("❌ Usage: /auth custom <json_file>");
                return;
            }
            match load_custom_provider(parts[1]) {
                Ok(p) => p,
                Err(e) => {
                    println!("❌ Failed to load provider config: {}", e);
                    return;
                }
            }
        }
        _ => {
            println!("❌ Unknown provider: {}", parts[0]);
            println!("   Supported: github, google, custom");
            return;
        }
    };

    match oauth::run_oauth_flow(&provider, store).await {
        Ok(_token) => {
            println!("\n🎉 OAuth authentication successful for {}!", provider.name);
        }
        Err(e) => {
            println!("\n❌ OAuth flow failed: {}", e);
        }
    }
}

/// Handle `/tokens` command — list all stored OAuth tokens.
fn handle_tokens_command(store: &TokenStore) {
    let providers = store.list_providers();
    if providers.is_empty() {
        println!("\n🔐 No OAuth tokens stored.");
        println!("   Use /auth <provider> to authenticate.");
        return;
    }

    println!("\n🔐 Stored OAuth tokens:");
    for name in providers {
        if let Some(token_set) = store.get(name) {
            let status = if token_set.is_expired() {
                "⚠️  expired"
            } else {
                "✅ valid"
            };
            let expiry = token_set
                .expires_at
                .map(|e| e.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "no expiry".to_string());
            let refresh = if token_set.refresh_token.is_some() {
                "has refresh token"
            } else {
                "no refresh token"
            };
            let scopes = if token_set.scopes.is_empty() {
                "no scopes".to_string()
            } else {
                token_set.scopes.join(", ")
            };
            println!(
                "   {} ({}) — {} | expires: {} | {} | scopes: {}",
                name, token_set.provider, status, expiry, refresh, scopes
            );
        }
    }
}

/// Handle `/revoke <provider>` command — remove stored tokens.
fn handle_revoke_command(input: &str, store: &mut TokenStore) {
    let provider = input.strip_prefix("/revoke ").unwrap().trim().to_lowercase();
    if provider.is_empty() {
        println!("❌ Usage: /revoke <provider>");
        return;
    }

    match store.remove(&provider) {
        Ok(()) => println!("✅ Tokens removed for: {}", provider),
        Err(e) => println!("❌ Failed to remove tokens: {}", e),
    }
}

/// Load a custom OAuth provider configuration from a JSON file.
fn load_custom_provider(path: &str) -> Result<OAuthProvider, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path, e))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {}", path, e))
}
