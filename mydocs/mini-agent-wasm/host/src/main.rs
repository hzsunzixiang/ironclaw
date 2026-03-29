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

use std::io::{self, Write};
use std::sync::Arc;

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

    println!("🔧 Loading WASM tool from: {}", wasm_path);
    let wasm_engine = match WasmToolEngine::new(&wasm_path, "calculator", 1_000_000) {
        Ok(engine) => {
            println!("✅ WASM tool loaded and compiled");
            println!("   Name: {}", engine.tool_name);
            println!("   Description: {}", engine.tool_description);
            println!("   Fuel limit: {} units per execution", engine.fuel_limit);
            engine
        }
        Err(e) => {
            eprintln!("❌ Failed to load WASM tool: {}", e);
            eprintln!("   Make sure to build the guest first:");
            eprintln!("   cd ../guest && cargo build --target wasm32-wasip2 --release");
            std::process::exit(1);
        }
    };
    println!();

    // ── Step 2: Setup LLM provider ──
    let hai_config = HaiConfig::load().unwrap_or_else(|e| {
        eprintln!("❌ Failed to load ~/HAI_WOA.json: {}", e);
        eprintln!("   Please create HAI_WOA.json in the current directory or ~/HAI_WOA.json.");
        std::process::exit(1);
    });

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

    let make_provider =
        |model: &str, config: &HaiConfig| -> Result<Box<dyn LlmProvider>, String> {
            Ok(Box::new(OpenAiCompatibleProvider::new(
                config.api_key()?,
                config.base_url()?,
                model.to_string(),
            )))
        };

    let mut llm: Box<dyn LlmProvider> = make_provider(&current_model, &hai_config)
        .unwrap_or_else(|e| {
            eprintln!("❌ Failed to create LLM provider: {}", e);
            std::process::exit(1);
        });

    println!();

    // ── Step 3: Register WASM tool ──
    // Instead of: registry.register(Box::new(CalculatorTool))  // native Rust
    // We do:      registry.register(Box::new(WasmTool { ... })) // WASM sandbox
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(WasmTool {
        engine: Arc::new(wasm_engine),
    }));

    println!("✅ Tool registry ready (WASM sandboxed tools)");
    for def in registry.definitions() {
        println!("   📦 {} — {}", def.name, def.description);
    }
    println!();

    // ── Step 4: System prompt ──
    let system_prompt = ChatMessage::system(
        "You are a helpful assistant with access to a calculator tool. \
         When the user asks a math question, use the calculator tool to compute the answer. \
         For non-math questions, respond directly.",
    );

    let config = AgenticLoopConfig::default();

    // ── Step 5: Main loop ──
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

        // ── Build conversation and run agentic loop ──
        let mut messages = vec![system_prompt.clone(), ChatMessage::user(input)];

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
