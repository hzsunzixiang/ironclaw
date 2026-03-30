//! # Mini Agent Loop with WASM Sandbox — IronClaw Core Distilled
//!
//! This builds on mini-agent-loop by adding WASM sandbox execution for tools.
//! Instead of tools being native Rust code, they run inside a Wasmtime WASM sandbox.
//!
//! ```text
//! stdin → Agentic Loop → LLM → WASM Sandbox Tool Execute → Response → stdout
//! ```
//!
//! ## Module Structure:
//!
//! | Module              | IronClaw source                          | Purpose                        |
//! |---------------------|------------------------------------------|--------------------------------|
//! | `llm::provider`     | `src/llm/provider.rs`                    | LLM abstraction trait          |
//! | `llm` (types)       | `src/llm/provider.rs`                    | ChatMessage, ToolCall, etc.    |
//! | `llm::openai`       | (replaces real provider configs)         | OpenAI-compatible provider     |
//! | `tools`             | `src/tools/tool.rs` + `src/wasm/sandbox` | Tool trait & WASM sandbox      |
//! | `agent`             | `src/agent/agentic_loop.rs`              | The core loop engine           |
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Host Process                             │
//! │                                                                 │
//! │  ┌──────────┐    ┌──────────┐    ┌──────────────────────────┐  │
//! │  │  stdin    │───▶│  Agent   │───▶│  LLM (DeepSeek/Qwen)    │  │
//! │  │  stdout   │◀──│  Loop    │◀──│  via ~/HAI_WOA.json      │  │
//! │  └──────────┘    └────┬─────┘    └──────────────────────────┘  │
//! │                       │                                         │
//! │                       │ tool_call(calculator, params)           │
//! │                       ▼                                         │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │              WASM Sandbox (Wasmtime)                      │  │
//! │  │  ┌────────────────────────────────────────────────────┐  │  │
//! │  │  │  Guest Tool (.wasm)                                │  │  │
//! │  │  │                                                    │  │  │
//! │  │  │  • Can ONLY call host::log() and host::now_millis()│  │  │
//! │  │  │  • Cannot access filesystem, network, env vars     │  │  │
//! │  │  │  • Fuel-limited (prevents infinite loops)          │  │  │
//! │  │  │  • Fresh instance per execution (no state leak)    │  │  │
//! │  │  └────────────────────────────────────────────────────┘  │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## How to run:
//!
//! ```bash
//! # 1. Build the guest WASM tool
//! cd guest && cargo build --target wasm32-wasip2 --release && cd ..
//!
//! # 2. Run the host (agent loop + WASM sandbox)
//! cd host && cargo run --release
//! ```

mod agent;
mod llm;
mod tools;

use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use tracing::{debug, info, trace, warn, error};

use agent::{run_agentic_loop, AgenticLoopConfig, LoopOutcome};
use llm::{ChatMessage, HaiConfig, LlmProvider, OpenAiCompatibleProvider};
use tools::{ToolRegistry, WasmTool, WasmToolEngine};

// ============================================================================
// Main — The Entry Point
// ============================================================================
// The key difference from mini-agent-loop: instead of registering a native
// CalculatorTool, we load a .wasm file and wrap it as a WasmTool.
// ============================================================================

#[tokio::main]
async fn main() {
    // ── Initialize tracing subscriber ──
    // Control log level via RUST_LOG env var:
    //   RUST_LOG=mini_agent_wasm=trace  — maximum detail for OUR code only (recommended)
    //   RUST_LOG=mini_agent_wasm=debug  — detailed (requests, responses, tool calls)
    //   RUST_LOG=info                   — key events only (default)
    //   RUST_LOG=mini_agent_wasm::llm=trace  — only LLM module at trace level
    //
    // ⚠️  Avoid RUST_LOG=trace (without module filter) — wasmtime internals
    //    (type_registry, cranelift, regalloc) produce thousands of unreadable lines.
    //    Always scope trace/debug to our crate: mini_agent_wasm=trace
    //
    // Default filter: our crate at info, third-party noisy crates at warn
    let default_filter = "mini_agent_wasm=info,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn,info";
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .with_target(true)       // show module path (e.g. mini_agent_wasm::llm::openai)
        .with_thread_names(true) // show thread names
        .with_level(true)        // show log level
        .with_ansi(std::io::stderr().is_terminal()) // auto-detect: colors in terminal, plain text in file
        .init();

    info!("🚀 Starting Mini Agent Loop + WASM Sandbox — tracing initialized");

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║      Mini Agent Loop + WASM Sandbox — IronClaw Distilled   ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║  This demo shows the core agentic loop with WASM sandbox:  ║");
    println!("║    stdin → LLM → WASM Sandbox Tool → Response → stdout     ║");
    println!("║                                                            ║");
    println!("║  Tools run inside a Wasmtime WASM sandbox:                 ║");
    println!("║    • Fuel-limited (prevents infinite loops)                ║");
    println!("║    • No filesystem/network access                          ║");
    println!("║    • Can only call host::log() and host::now_millis()      ║");
    println!("║    • Fresh instance per execution (no state leak)          ║");
    println!("║                                                            ║");
    println!("║  Config: ./HAI_WOA.json (or ~/HAI_WOA.json)                ║");
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

    // ── Step 1: Load WASM tool ──
    // In IronClaw: WASM tools are loaded from a configured directory
    // Here we look for the guest .wasm in a known location
    let wasm_path = std::env::args().nth(1).unwrap_or_else(|| {
        "../guest/target/wasm32-wasip2/release/guest_tool.wasm".to_string()
    });

    info!(wasm_path = %wasm_path, "🔧 Loading WASM tool...");
    println!("🔧 Loading WASM tool from: {}", wasm_path);
    let wasm_engine = match WasmToolEngine::new(&wasm_path, 1_000_000) {
        Ok(engine) => {
            info!(
                tool_name = %engine.tool_name,
                tool_description = %engine.tool_description,
                fuel_limit = engine.fuel_limit,
                "✅ WASM tool loaded and compiled"
            );
            debug!(
                tool_schema = %serde_json::to_string_pretty(&engine.tool_schema).unwrap_or_default(),
                "WASM tool schema"
            );
            println!("✅ WASM tool loaded and compiled");
            println!("   Name: {}", engine.tool_name);
            println!("   Description: {}", engine.tool_description);
            println!("   Fuel limit: {} units per execution", engine.fuel_limit);
            engine
        }
        Err(e) => {
            error!(error = %e, "❌ Failed to load WASM tool");
            eprintln!("❌ Failed to load WASM tool: {}", e);
            eprintln!("   Make sure to build the guest first:");
            eprintln!("   cd ../guest && cargo build --target wasm32-wasip2 --release");
            std::process::exit(1);
        }
    };
    println!();

    // ── Step 2: Setup LLM provider ──
    info!("📂 Loading LLM configuration from HAI_WOA.json ...");
    let hai_config = HaiConfig::load().unwrap_or_else(|e| {
        error!("❌ Failed to load HAI_WOA.json: {}", e);
        eprintln!("❌ Failed to load ~/HAI_WOA.json: {}", e);
        eprintln!("   Please create HAI_WOA.json in the current directory or ~/HAI_WOA.json.");
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

    println!("✅ Loaded config from {}",
        if std::path::Path::new("HAI_WOA.json").exists() { "./HAI_WOA.json" } else { "~/HAI_WOA.json" }
    );
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

    let make_provider =
        |model: &str, config: &HaiConfig| -> Result<Box<dyn LlmProvider>, String> {
            debug!(model = %model, "Creating new LLM provider instance");
            Ok(Box::new(OpenAiCompatibleProvider::new(
                config.api_key()?,
                config.base_url()?,
                model.to_string(),
            )))
        };

    let mut llm: Box<dyn LlmProvider> = make_provider(&current_model, &hai_config)
        .unwrap_or_else(|e| {
            error!("❌ Failed to create LLM provider: {}", e);
            eprintln!("❌ Failed to create LLM provider: {}", e);
            std::process::exit(1);
        });

    println!();

    // ── Step 3: Register WASM tool ──
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(WasmTool {
        engine: Arc::new(wasm_engine),
    }));

    info!(
        tools = ?registry.definitions().iter().map(|t| &t.name).collect::<Vec<_>>(),
        "🔧 Tool registry initialized (WASM sandboxed tools)"
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

    println!("✅ Tool registry ready (WASM sandboxed tools)");
    for def in registry.definitions() {
        println!("   📦 {} — {}", def.name, def.description);
    }
    println!();

    // ── Step 4: System prompt ──
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

    let config = AgenticLoopConfig::default();
    info!(max_iterations = config.max_iterations, "⚙️  Agentic loop config");

    // ── Step 5: Main loop ──
    info!("🔄 Entering main input loop — waiting for user input...");

    loop {
        print!("\n🧑 You: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => break,
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

        // ── Build conversation and run agentic loop ──
        info!("📋 Building conversation context (system prompt + user message)");
        let mut messages = vec![make_system_prompt(&current_model), ChatMessage::user(input)];
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
        output.push_str(&format!("\n  [{}] {} | content: \"{}\"", i, role, &msg.content));
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
