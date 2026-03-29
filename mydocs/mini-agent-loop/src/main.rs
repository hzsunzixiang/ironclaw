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

    // 3. Helper to build system prompt with current model identity
    let make_system_prompt = |model: &str| -> ChatMessage {
        ChatMessage::system(format!(
            "You are a helpful assistant powered by the {} model. \
             You have access to a calculator tool. \
             When the user asks a math question, use the calculator tool to compute the answer. \
             For non-math questions, respond directly. \
             When asked about your identity, truthfully state that you are based on {}.",
            model, model
        ))
    };

    // 4. Loop config
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

        // ── Build conversation context ──
        let mut messages = vec![make_system_prompt(&current_model), ChatMessage::user(input)];

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
