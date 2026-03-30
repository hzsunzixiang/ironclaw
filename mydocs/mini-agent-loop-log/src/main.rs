//! # Mini Agent Loop — IronClaw Core Distilled
//!
//! This is a minimal, standalone extraction of IronClaw's agentic loop.
//! It demonstrates the complete message flow:
//!
//! ```text
//! stdin → Agentic Loop → LLM → Tool Execute → Response → stdout
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
//!
//! ## How to run:
//!
//! ```bash
//! cargo run
//! ```

mod agent;
mod llm;
mod tools;

use std::io::{self, Write};

use tracing::{debug, info, trace, warn, error};

use agent::{run_agentic_loop, AgenticLoopConfig, LoopOutcome};
use llm::{ChatMessage, HaiConfig, LlmProvider, OpenAiCompatibleProvider};
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
// ============================================================================

#[tokio::main]
async fn main() {
    // ── Initialize tracing subscriber ──
    // Control log level via RUST_LOG env var:
    //   RUST_LOG=trace cargo run    — maximum detail (every variable, every step)
    //   RUST_LOG=debug cargo run    — detailed (requests, responses, tool calls)
    //   RUST_LOG=info  cargo run    — key events only (default)
    //   RUST_LOG=mini_agent_loop::llm=trace cargo run  — only LLM module at trace level
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(true)       // show module path (e.g. mini_agent_loop::llm::openai)
        .with_thread_names(true) // show thread names
        .with_level(true)        // show log level
        .init();

    info!("🚀 Starting Mini Agent Loop — tracing initialized");

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           Mini Agent Loop — IronClaw Core Distilled         ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║  This demo shows the core agentic loop:                    ║");
    println!("║    stdin → LLM → Tool Execute → Response → stdout          ║");
    println!("║                                                            ║");
    println!("║  Config: ./HAI_WOA.json (real LLM API)                     ║");
    println!("║                                                            ║");
    println!("║  Try:                                                      ║");
    println!("║    • \"What is 42 + 58?\"                                    ║");
    println!("║    • \"Calculate 100 divided by 3\"                          ║");
    println!("║    • \"Hello\" (direct response, no tool)                    ║");
    println!("║    • \"/model <name>\" to switch model (ds, qwen, kimi...)   ║");
    println!("║    • \"/models\" to list available models                    ║");
    println!("║    • \"quit\" to exit                                        ║");
    println!("║                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // ── Setup (corresponds to Agent::new() in IronClaw) ──

    // 1. Create LLM provider from ./HAI_WOA.json config
    info!("📂 Loading LLM configuration from ./HAI_WOA.json ...");
    let hai_config = HaiConfig::load().unwrap_or_else(|e| {
        error!("❌ Failed to load ./HAI_WOA.json: {}", e);
        eprintln!("❌ Failed to load ./HAI_WOA.json: {}", e);
        eprintln!("   Please copy HAI_WOA.template.json to HAI_WOA.json and fill in your API key.");
        std::process::exit(1);
    });

    info!(
        model = %hai_config.model,
        base_url = %hai_config.base_url().unwrap_or_default(),
        "✅ Config loaded successfully"
    );
    debug!(
        available_models = ?hai_config.list_models(),
        "Available model shortcuts"
    );

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
    info!(current_model = %current_model, "🤖 Initial model selected");

    let make_provider = |model: &str, config: &HaiConfig| -> Result<Box<dyn LlmProvider>, String> {
        debug!(model = %model, "Creating new LLM provider instance");
        Ok(Box::new(OpenAiCompatibleProvider::new(
            config.api_key()?,
            config.base_url()?,
            model.to_string(),
        )))
    };

    let mut llm: Box<dyn LlmProvider> =
        make_provider(&current_model, &hai_config).unwrap_or_else(|e| {
            error!("❌ Failed to create LLM provider: {}", e);
            eprintln!("❌ Failed to create LLM provider: {}", e);
            std::process::exit(1);
        });

    println!();

    // 2. Create tool registry and register tools
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CalculatorTool));
    info!(
        tools = ?registry.definitions().iter().map(|t| &t.name).collect::<Vec<_>>(),
        "🔧 Tool registry initialized"
    );
    debug!(
        tool_definitions = %serde_json::to_string_pretty(&registry.definitions().iter().map(|t| {
            serde_json::json!({
                "name": &t.name,
                "description": &t.description,
                "parameters": &t.parameters,
            })
        }).collect::<Vec<_>>()).unwrap_or_default(),
        "Full tool definitions (sent to LLM)"
    );

    // 3. Helper to build system prompt (model-agnostic)
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

    // 4. Loop config
    let config = AgenticLoopConfig::default();
    info!(max_iterations = config.max_iterations, "⚙️  Agentic loop config");

    // ── Main Loop (corresponds to Agent::run() event loop) ──
    info!("🔄 Entering main input loop — waiting for user input...");

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
            trace!("Empty input, skipping");
            continue;
        }
        if input == "quit" || input == "exit" {
            info!("👋 User requested exit");
            println!("👋 Goodbye!");
            break;
        }

        info!(input = %input, "📝 User input received");

        // ── Handle slash commands ──
        if input == "/models" {
            debug!("Listing available models");
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
            info!(
                requested = %model_name,
                resolved = %resolved,
                "🔄 Switching model"
            );
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

        // ── Build conversation context ──
        info!("📋 Building conversation context (system prompt + user message)");
        let mut messages = vec![make_system_prompt(), ChatMessage::user(input)];
        debug!(
            system_prompt = %messages[0].content,
            user_message = %messages[1].content,
            message_count = messages.len(),
            "Initial messages for agentic loop"
        );
        trace!(
            messages_detail = %format_messages_for_log(&messages),
            "Full message context before agentic loop"
        );

        // ── Run the agentic loop ──
        info!("🧠 Starting agentic loop (model={}, max_iterations={})", current_model, config.max_iterations);
        println!("\n🤖 Agent thinking...");

        let start_time = std::time::Instant::now();
        match run_agentic_loop(llm.as_ref(), &registry, &mut messages, &config).await {
            Ok(LoopOutcome::Response(text)) => {
                let elapsed = start_time.elapsed();
                info!(
                    elapsed_ms = elapsed.as_millis(),
                    response_len = text.len(),
                    "✅ Agentic loop completed with text response"
                );
                debug!(response = %text, "Full agent response");
                trace!(
                    final_messages = %format_messages_for_log(&messages),
                    "Final message context after agentic loop"
                );
                println!("\n🤖 Agent: {}", text);
            }
            Ok(LoopOutcome::MaxIterations) => {
                let elapsed = start_time.elapsed();
                warn!(
                    elapsed_ms = elapsed.as_millis(),
                    "⚠️  Agentic loop reached max iterations without final response"
                );
                trace!(
                    final_messages = %format_messages_for_log(&messages),
                    "Message context at max iterations"
                );
                println!("\n⚠️  Agent: Reached maximum iterations without a final response.");
            }
            Err(e) => {
                let elapsed = start_time.elapsed();
                error!(
                    elapsed_ms = elapsed.as_millis(),
                    error = %e,
                    "❌ Agentic loop failed with error"
                );
                println!("\n❌ Error: {}", e);
            }
        }
    }
}

/// Format messages for detailed logging
fn format_messages_for_log(messages: &[ChatMessage]) -> String {
    let mut output = String::new();
    for (i, msg) in messages.iter().enumerate() {
        let role = match msg.role {
            llm::Role::System => "SYSTEM",
            llm::Role::User => "USER",
            llm::Role::Assistant => "ASSISTANT",
            llm::Role::Tool => "TOOL",
        };
        output.push_str(&format!("\n  [{}] {} | content: \"{}\"", i, role, truncate_for_log(&msg.content, 200)));
        if let Some(ref tc) = msg.tool_calls {
            output.push_str(&format!(" | tool_calls: {:?}", tc));
        }
        if let Some(ref id) = msg.tool_call_id {
            output.push_str(&format!(" | tool_call_id: {}", id));
        }
        if let Some(ref name) = msg.name {
            output.push_str(&format!(" | name: {}", name));
        }
    }
    output
}

/// Truncate string for log display
fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...[truncated, total {} bytes]", &s[..end], s.len())
    }
}
