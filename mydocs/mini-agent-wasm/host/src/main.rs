//! # Mini Agent Loop with WASM Sandbox — IronClaw Core Distilled
//!
//! This builds on mini-agent-loop by adding WASM sandbox execution for tools.
//! Instead of tools being native Rust code, they run inside a Wasmtime WASM sandbox.
//!
//! ```text
//! stdin → Agentic Loop → LLM → WASM Sandbox Tool Execute → Response → stdout
//! ```
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
//! ## What's different from mini-agent-loop:
//!
//! | mini-agent-loop                | mini-agent-wasm                          |
//! |-------------------------------|------------------------------------------|
//! | `CalculatorTool` is native Rust | `CalculatorTool` runs in WASM sandbox  |
//! | `Tool` trait with `execute()`  | `WasmTool` wraps Wasmtime component     |
//! | No isolation                   | Full sandbox: fuel, no FS/net, log only |
//! | Single binary                  | host binary + guest .wasm component     |
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

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wasmtime::component::Linker;
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

// ============================================================================
// PART 1: LLM Types & Provider Trait
// ============================================================================
// Same as mini-agent-loop — the LLM layer is unchanged.
// The WASM sandbox only affects the tool execution layer.
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), tool_call_id: None, name: None, tool_calls: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), tool_call_id: None, name: None, tool_calls: None }
    }
    pub fn assistant_with_tool_calls(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.unwrap_or_default(),
            tool_call_id: None,
            name: None,
            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
        }
    }
    pub fn tool_result(call_id: impl Into<String>, name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(call_id.into()),
            name: Some(name.into()),
            tool_calls: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FinishReason {
    Stop,
    ToolUse,
    Length,
}

#[derive(Debug)]
pub enum LlmOutput {
    Text(String),
    ToolCalls {
        tool_calls: Vec<ToolCall>,
        content: Option<String>,
    },
}

#[derive(Debug)]
pub struct LlmResponse {
    pub result: LlmOutput,
    pub finish_reason: FinishReason,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse, String>;
}

// ============================================================================
// PART 2: Tool Trait & WASM Sandbox Execution
// ============================================================================
// THIS IS THE KEY DIFFERENCE from mini-agent-loop.
//
// In mini-agent-loop: Tool trait → native Rust execute()
// Here:              Tool trait → WasmTool → Wasmtime sandbox → guest .wasm
//
// The execution pipeline becomes:
//   1. LLM returns tool_call(calculator, params)
//   2. execute_tool_with_safety() looks up the tool
//   3. WasmTool::execute() creates a fresh WASM Store
//   4. Guest .wasm runs in sandbox with fuel limit
//   5. Guest can ONLY call host::log() and host::now_millis()
//   6. Result comes back through the WIT interface
//
// This maps to IronClaw's:
//   src/tools/execute.rs → execute_tool_with_safety()
//   src/wasm/sandbox.rs  → WasmSandbox::execute()
// ============================================================================

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub result: serde_json::Value,
}

/// Trait for tools that the agent can use.
/// Both native and WASM tools implement this trait.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String>;
}

/// Tool registry — holds all available tools.
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters_schema(),
        }).collect()
    }
}

// ════════════════════════════════════════════════════════════════════
// WASM Sandbox Infrastructure
// ════════════════════════════════════════════════════════════════════
// This section implements the WASM sandbox that tools run inside.
//
// Key components:
//   1. WIT bindings (generated by wasmtime::component::bindgen!)
//   2. StoreData — per-execution state (logs, WASI context)
//   3. Host trait impl — what the guest can call (log, now_millis)
//   4. WasmToolEngine — shared engine + compiled component
//   5. WasmTool — implements Tool trait using the sandbox
// ════════════════════════════════════════════════════════════════════

// Step 1: Generate host-side bindings from the WIT file.
//
// This macro reads the WIT and generates:
//   - `demo::sandbox::host::Host` trait (we must implement)
//   - `demo::sandbox::host::add_to_linker()` function
//   - `SandboxedTool` struct with `instantiate()` to create guest instances
//   - `exports::demo::sandbox::tool::*` types for calling guest functions
wasmtime::component::bindgen!({
    path: "../wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});

// Step 2: Per-execution state.
//
// Each tool execution gets a FRESH Store — this is the "fresh instance
// per execution" pattern, ensuring complete isolation between runs.
// No state leaks from one execution to the next.
struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    /// Collected log messages from the guest
    logs: Vec<(String, String)>,
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// Step 3: Implement the Host trait — THE CORE OF THE SANDBOX.
//
// When the guest calls `host::log(...)` or `host::now_millis()`,
// execution crosses the WASM boundary and arrives HERE.
//
// The host has FULL CONTROL over what these functions do:
// - log() could write to a file, send to a server, or just collect in memory
// - now_millis() could return the real time, or a fake time for testing
// - In IronClaw, http_request() checks an allowlist before making the request
//
// The guest has NO WAY to bypass this — it's enforced by the WASM VM.
impl demo::sandbox::host::Host for StoreData {
    fn log(&mut self, level: demo::sandbox::host::LogLevel, message: String) {
        let level_str = match level {
            demo::sandbox::host::LogLevel::Info => "INFO",
            demo::sandbox::host::LogLevel::Warn => "WARN",
            demo::sandbox::host::LogLevel::Error => "ERROR",
        };
        println!("    📋 [WASM LOG] [{level_str}] {message}");
        self.logs.push((level_str.to_string(), message));
    }

    fn now_millis(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

// Step 4: Shared WASM engine + compiled component.
//
// The Engine and compiled Component are expensive to create but can be
// reused across executions. Only the Store (per-execution state) is fresh.
struct WasmToolEngine {
    engine: Engine,
    component: wasmtime::component::Component,
    /// Tool metadata (cached from first instantiation)
    tool_name: String,
    tool_description: String,
    tool_schema: serde_json::Value,
    /// Fuel limit per execution
    fuel_limit: u64,
}

impl WasmToolEngine {
    /// Create a new WASM tool engine from a .wasm file.
    ///
    /// This:
    /// 1. Creates a Wasmtime engine with component model + fuel metering
    /// 2. Compiles the WASM bytes to native code
    /// 3. Instantiates once to read tool metadata (name, description, schema)
    fn new(wasm_path: &str, tool_name: &str, fuel_limit: u64) -> Result<Self, String> {
        // Create engine with component model and fuel metering
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|e| format!("Failed to create WASM engine: {}", e))?;

        // Load and compile the WASM component
        let wasm_bytes = std::fs::read(wasm_path)
            .map_err(|e| format!("Failed to read WASM file '{}': {}", wasm_path, e))?;
        let component = wasmtime::component::Component::new(&engine, &wasm_bytes)
            .map_err(|e| format!("Failed to compile WASM component: {}", e))?;

        // Instantiate once to read metadata
        let mut linker: Linker<StoreData> = Linker::new(&engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("Failed to add WASI to linker: {}", e))?;
        demo::sandbox::host::add_to_linker(&mut linker, |state| state)
            .map_err(|e| format!("Failed to add host functions to linker: {}", e))?;

        let mut store = Store::new(
            &engine,
            StoreData {
                wasi: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
                logs: Vec::new(),
            },
        );
        store.set_fuel(fuel_limit).map_err(|e| format!("Failed to set fuel: {}", e))?;

        let instance = SandboxedTool::instantiate(&mut store, &component, &linker)
            .map_err(|e| format!("Failed to instantiate WASM tool: {}", e))?;

        let description = instance.demo_sandbox_tool().call_description(&mut store)
            .map_err(|e| format!("Failed to call description(): {}", e))?;
        let schema_str = instance.demo_sandbox_tool().call_schema(&mut store)
            .map_err(|e| format!("Failed to call schema(): {}", e))?;
        let schema: serde_json::Value = serde_json::from_str(&schema_str)
            .map_err(|e| format!("Failed to parse schema JSON: {}", e))?;

        Ok(Self {
            engine,
            component,
            tool_name: tool_name.to_string(),
            tool_description: description,
            tool_schema: schema,
            fuel_limit,
        })
    }

    /// Execute the tool in a fresh sandbox.
    ///
    /// Each call creates a NEW Store — complete isolation between executions.
    /// The guest gets a fresh fuel budget and clean state every time.
    fn execute_in_sandbox(&self, params: &str) -> Result<String, String> {
        // Fresh linker for this execution
        let mut linker: Linker<StoreData> = Linker::new(&self.engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("WASI linker error: {}", e))?;
        demo::sandbox::host::add_to_linker(&mut linker, |state| state)
            .map_err(|e| format!("Host linker error: {}", e))?;

        // Fresh store — complete isolation from previous executions
        let mut store = Store::new(
            &self.engine,
            StoreData {
                wasi: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
                logs: Vec::new(),
            },
        );
        store.set_fuel(self.fuel_limit).map_err(|e| format!("Fuel error: {}", e))?;

        // Instantiate the guest in the sandbox
        let instance = SandboxedTool::instantiate(&mut store, &self.component, &linker)
            .map_err(|e| format!("Instantiation error: {}", e))?;

        // Call the guest's execute() function
        let request = exports::demo::sandbox::tool::Request {
            params: params.to_string(),
        };
        let response = instance.demo_sandbox_tool().call_execute(&mut store, &request)
            .map_err(|e| format!("WASM execution error: {}", e))?;

        // Report fuel consumption
        let remaining = store.get_fuel().unwrap_or(0);
        let consumed = self.fuel_limit - remaining;
        println!("    ⛽ Fuel consumed: {} / {} units", consumed, self.fuel_limit);

        // Check response
        if let Some(error) = response.error {
            return Err(format!("Tool error: {}", error));
        }
        response.output.ok_or_else(|| "Tool returned no output".to_string())
    }
}

// Step 5: WasmTool — implements the Tool trait using the WASM sandbox.
//
// This is the bridge between the agent loop's Tool abstraction and
// the WASM sandbox execution. The agent loop doesn't know or care
// that the tool runs in a sandbox — it just calls execute().
struct WasmTool {
    engine: Arc<WasmToolEngine>,
}

#[async_trait]
impl Tool for WasmTool {
    fn name(&self) -> &str {
        &self.engine.tool_name
    }

    fn description(&self) -> &str {
        &self.engine.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.engine.tool_schema.clone()
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let params_str = serde_json::to_string(&params)
            .map_err(|e| format!("Failed to serialize params: {}", e))?;

        // Execute in WASM sandbox (synchronous — WASM execution is sync)
        // We use spawn_blocking to avoid blocking the async runtime
        let engine = self.engine.clone();
        let result = tokio::task::spawn_blocking(move || {
            engine.execute_in_sandbox(&params_str)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))??;

        // Parse the JSON output from the guest
        let value: serde_json::Value = serde_json::from_str(&result)
            .map_err(|e| format!("Failed to parse tool output: {}", e))?;

        Ok(ToolOutput { result: value })
    }
}

// ════════════════════════════════════════════════════════════════════
// Tool Execution Pipeline
// ════════════════════════════════════════════════════════════════════

/// Execute a tool with timeout.
/// Maps to: `src/tools/execute.rs` → `execute_tool_with_safety()`
async fn execute_tool_with_safety(
    registry: &ToolRegistry,
    tool_name: &str,
    params: serde_json::Value,
) -> Result<String, String> {
    let tool = registry.get(tool_name)
        .ok_or_else(|| format!("Tool '{}' not found", tool_name))?;

    println!("  ⚙️  Executing tool: {} with params: {}", tool_name, params);
    println!("    🔒 Running in WASM sandbox...");

    let timeout = Duration::from_secs(30);
    let result = tokio::time::timeout(timeout, tool.execute(params)).await;

    match result {
        Ok(Ok(output)) => {
            serde_json::to_string_pretty(&output.result)
                .map_err(|e| format!("Failed to serialize result: {}", e))
        }
        Ok(Err(e)) => Err(format!("Tool execution failed: {}", e)),
        Err(_) => Err(format!("Tool '{}' timed out after {:?}", tool_name, timeout)),
    }
}

fn process_tool_result(
    tool_name: &str,
    tool_call_id: &str,
    result: &Result<String, String>,
) -> ChatMessage {
    let content = match result {
        Ok(output) => format!("<tool_output>{}</tool_output>", output),
        Err(e) => format!("Error: {}", e),
    };
    ChatMessage::tool_result(tool_call_id, tool_name, content)
}

// ============================================================================
// PART 3: Agentic Loop — The Core Engine
// ============================================================================
// Same as mini-agent-loop — the loop doesn't change.
// It calls tools through the Tool trait, unaware of WASM underneath.
// ============================================================================

pub struct AgenticLoopConfig {
    pub max_iterations: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self { max_iterations: 10 }
    }
}

pub enum LoopOutcome {
    Response(String),
    MaxIterations,
}

async fn run_agentic_loop(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = registry.definitions();

    for iteration in 1..=config.max_iterations {
        println!("\n--- Iteration {}/{} ---", iteration, config.max_iterations);

        let response = llm.chat(messages, &tool_defs).await?;

        match response.result {
            LlmOutput::Text(text) => {
                println!("  💬 LLM returned text: {}", &text[..text.len().min(100)]);
                return Ok(LoopOutcome::Response(text));
            }

            LlmOutput::ToolCalls { tool_calls, content } => {
                println!("  🔧 LLM wants to call {} tool(s)", tool_calls.len());

                if response.finish_reason == FinishReason::Length {
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        messages.push(ChatMessage {
                            role: Role::Assistant,
                            content: text,
                            tool_call_id: None,
                            name: None,
                            tool_calls: None,
                        });
                    }
                    messages.push(ChatMessage::user(
                        "Your previous response was truncated. Please try a simpler approach."
                    ));
                    continue;
                }

                messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                for tc in &tool_calls {
                    let result = execute_tool_with_safety(registry, &tc.name, tc.arguments.clone()).await;

                    let result_msg = process_tool_result(&tc.name, &tc.id, &result);

                    match &result {
                        Ok(output) => println!("  ✅ Tool '{}' succeeded: {}",
                            tc.name, &output[..output.len().min(80)]),
                        Err(e) => println!("  ❌ Tool '{}' failed: {}", tc.name, e),
                    }

                    messages.push(result_msg);
                }
            }
        }
    }

    Ok(LoopOutcome::MaxIterations)
}

// ============================================================================
// PART 4: Mock LLM Provider (for testing without API key)
// ============================================================================

struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolDefinition],
    ) -> Result<LlmResponse, String> {
        let last_user = messages.iter().rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let last_msg = messages.last();
        if let Some(msg) = last_msg {
            if msg.role == Role::Tool {
                return Ok(LlmResponse {
                    result: LlmOutput::Text(format!(
                        "Based on the calculation result: {}",
                        msg.content
                            .replace("<tool_output>", "")
                            .replace("</tool_output>", "")
                            .trim()
                    )),
                    finish_reason: FinishReason::Stop,
                });
            }
        }

        let lower = last_user.to_lowercase();
        let is_math = lower.contains('+') || lower.contains('-') || lower.contains('*')
            || lower.contains('/') || lower.contains("plus") || lower.contains("minus")
            || lower.contains("times") || lower.contains("multiply")
            || lower.contains("divide") || lower.contains("divided")
            || lower.contains("add") || lower.contains("subtract")
            || lower.contains("calculate") || lower.contains("what is")
            || lower.contains("how much");

        if is_math {
            let (op, a, b) = parse_math_intent(last_user);
            Ok(LlmResponse {
                result: LlmOutput::ToolCalls {
                    tool_calls: vec![ToolCall {
                        id: "call_001".to_string(),
                        name: "calculator".to_string(),
                        arguments: serde_json::json!({
                            "operation": op,
                            "a": a,
                            "b": b,
                        }),
                    }],
                    content: None,
                },
                finish_reason: FinishReason::ToolUse,
            })
        } else {
            Ok(LlmResponse {
                result: LlmOutput::Text(format!(
                    "I'm a mini agent demo with WASM sandbox! I can do math! Try asking me \
                     something like 'What is 42 + 58?' or 'Calculate 100 divided by 3'. \
                     You said: \"{}\"",
                    last_user
                )),
                finish_reason: FinishReason::Stop,
            })
        }
    }
}

fn parse_math_intent(input: &str) -> (&str, f64, f64) {
    let lower = input.to_lowercase();
    let numbers: Vec<f64> = input
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    let a = numbers.first().copied().unwrap_or(0.0);
    let b = numbers.get(1).copied().unwrap_or(0.0);

    let op = if lower.contains('+') || lower.contains("plus") || lower.contains("add") {
        "add"
    } else if lower.contains('-') || lower.contains("minus") || lower.contains("subtract") {
        "sub"
    } else if lower.contains('*') || lower.contains("times") || lower.contains("multiply") {
        "mul"
    } else if lower.contains('/') || lower.contains("divide") {
        "div"
    } else {
        "add"
    };

    (op, a, b)
}

// ============================================================================
// PART 5: Real OpenAI-Compatible LLM Provider
// ============================================================================

#[derive(Debug, Deserialize)]
struct HaiConfig {
    model: String,
    #[serde(default)]
    models: HashMap<String, ModelEntry>,
    env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

impl HaiConfig {
    fn load() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
        let path = home.join("HAI_WOA.json");
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
    }

    fn api_key(&self) -> Result<String, String> {
        self.env.get("OPENAI_API_KEY").cloned()
            .ok_or_else(|| "OPENAI_API_KEY not found in HAI_WOA.json env".to_string())
    }

    fn base_url(&self) -> Result<String, String> {
        self.env.get("OPENAI_BASE_URL").cloned()
            .ok_or_else(|| "OPENAI_BASE_URL not found in HAI_WOA.json env".to_string())
    }

    fn resolve_model(&self, name: Option<&str>) -> String {
        let raw = match name {
            Some(n) => {
                if let Some(entry) = self.models.get(n) {
                    entry.id.clone()
                } else {
                    n.to_string()
                }
            }
            None => self.model.clone(),
        };
        raw.strip_prefix("openai/").unwrap_or(&raw).to_string()
    }

    fn list_models(&self) -> Vec<(&str, &str)> {
        let mut models: Vec<_> = self.models.iter()
            .map(|(k, v)| (k.as_str(), v.id.as_str()))
            .collect();
        models.sort_by_key(|(k, _)| *k);
        models
    }
}

struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiCompatibleProvider {
    fn new(api_key: String, base_url: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self { client, api_key, base_url, model }
    }
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCallOut>>,
}

#[derive(Serialize)]
struct OpenAiToolCallOut {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiFunctionCallOut,
}

#[derive(Serialize)]
struct OpenAiFunctionCallOut {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunction,
}

#[derive(Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize, Debug)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OpenAiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCallIn>>,
}

#[derive(Deserialize, Debug)]
struct OpenAiToolCallIn {
    id: String,
    function: OpenAiFunctionIn,
}

#[derive(Deserialize, Debug)]
struct OpenAiFunctionIn {
    name: String,
    arguments: String,
}

impl OpenAiCompatibleProvider {
    fn convert_messages(messages: &[ChatMessage]) -> Vec<OpenAiMessage> {
        messages.iter().map(|m| {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            let tool_calls_out = m.tool_calls.as_ref().map(|tcs| {
                tcs.iter().map(|tc| OpenAiToolCallOut {
                    id: tc.id.clone(),
                    call_type: "function".to_string(),
                    function: OpenAiFunctionCallOut {
                        name: tc.name.clone(),
                        arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    },
                }).collect()
            });
            OpenAiMessage {
                role: role.to_string(),
                content: m.content.clone(),
                tool_call_id: m.tool_call_id.clone(),
                name: m.name.clone(),
                tool_calls: tool_calls_out,
            }
        }).collect()
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<OpenAiTool> {
        tools.iter().map(|t| OpenAiTool {
            tool_type: "function".to_string(),
            function: OpenAiFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        }).collect()
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse, String> {
        let url = format!("{}/chat/completions", self.base_url);

        let openai_tools = Self::convert_tools(tools);
        let body = OpenAiRequest {
            model: self.model.clone(),
            messages: Self::convert_messages(messages),
            tools: openai_tools,
            tool_choice: if tools.is_empty() { None } else { Some("auto".to_string()) },
        };

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            return Err(format!("API returned {}: {}", status, error_body));
        }

        let openai_resp: OpenAiResponse = resp.json().await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        let choice = openai_resp.choices.into_iter().next()
            .ok_or("No choices in response")?;

        let finish = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("length") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        if let Some(tool_calls_in) = choice.message.tool_calls {
            if !tool_calls_in.is_empty() {
                let tool_calls: Vec<ToolCall> = tool_calls_in.into_iter().map(|tc| {
                    let arguments: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                    ToolCall {
                        id: tc.id,
                        name: tc.function.name,
                        arguments,
                    }
                }).collect();

                return Ok(LlmResponse {
                    result: LlmOutput::ToolCalls {
                        tool_calls,
                        content: choice.message.content,
                    },
                    finish_reason: FinishReason::ToolUse,
                });
            }
        }

        let text = choice.message.content.unwrap_or_default();
        Ok(LlmResponse {
            result: LlmOutput::Text(text),
            finish_reason: finish,
        })
    }
}

// ============================================================================
// PART 6: Main — Entry Point
// ============================================================================
// The key difference: instead of registering a native CalculatorTool,
// we load a .wasm file and wrap it as a WasmTool.
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
    println!("║  Config: ~/HAI_WOA.json (real LLM API)                     ║");
    println!("║                                                            ║");
    println!("║  Try:                                                      ║");
    println!("║    • \"What is 42 + 58?\"                                    ║");
    println!("║    • \"Calculate 100 divided by 3\"                          ║");
    println!("║    • \"Hello\" (direct response, no tool)                    ║");
    println!("║    • \"/model <name>\" to switch model (ds, qwen, kimi...)   ║");
    println!("║    • \"/models\" to list available models                    ║");
    println!("║    • \"/mock\" to switch to mock LLM (no API)               ║");
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
    let hai_config = match HaiConfig::load() {
        Ok(c) => {
            println!("✅ Loaded config from ~/HAI_WOA.json");
            println!("   Model: {}", c.model);
            println!("   Base URL: {}", c.base_url().unwrap_or_default());
            println!("   Available shortcuts: {}", c.list_models().iter()
                .map(|(k, v)| format!("{} → {}", k, v))
                .collect::<Vec<_>>().join(", "));
            Some(c)
        }
        Err(e) => {
            println!("⚠️  Failed to load ~/HAI_WOA.json: {}", e);
            println!("   Falling back to MockLlmProvider (no real API calls)");
            None
        }
    };

    let mut use_mock = hai_config.is_none();
    let mut current_model = hai_config.as_ref().map(|c| c.resolve_model(None)).unwrap_or_default();

    let make_real_provider = |model: &str, config: &HaiConfig| -> Result<Box<dyn LlmProvider>, String> {
        Ok(Box::new(OpenAiCompatibleProvider::new(
            config.api_key()?,
            config.base_url()?,
            model.to_string(),
        )))
    };

    let mut llm: Box<dyn LlmProvider> = if let Some(ref config) = hai_config {
        match make_real_provider(&current_model, config) {
            Ok(p) => p,
            Err(e) => {
                println!("⚠️  Failed to create provider: {}. Using mock.", e);
                use_mock = true;
                Box::new(MockLlmProvider)
            }
        }
    } else {
        Box::new(MockLlmProvider)
    };

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
         For non-math questions, respond directly."
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

        // Handle slash commands
        if input == "/models" {
            if let Some(ref config) = hai_config {
                println!("\n📋 Available models:");
                println!("   (default) → {}", config.model);
                for (shortcut, model_id) in config.list_models() {
                    let marker = if current_model == model_id { " ← current" } else { "" };
                    println!("   {} → {}{}", shortcut, model_id, marker);
                }
                println!("   Current: {}{}", current_model, if use_mock { " (MOCK)" } else { "" });
            } else {
                println!("\n⚠️  No config loaded. Using mock LLM.");
            }
            continue;
        }
        if input.starts_with("/model ") {
            let model_name = input.strip_prefix("/model ").unwrap().trim();
            if let Some(ref config) = hai_config {
                let resolved = config.resolve_model(Some(model_name));
                current_model = resolved.clone();
                use_mock = false;
                match make_real_provider(&resolved, config) {
                    Ok(p) => {
                        llm = p;
                        println!("✅ Switched to model: {}", resolved);
                    }
                    Err(e) => println!("❌ Failed to switch model: {}", e),
                }
            } else {
                println!("⚠️  No config loaded. Cannot switch model.");
            }
            continue;
        }
        if input == "/mock" {
            use_mock = true;
            llm = Box::new(MockLlmProvider);
            println!("✅ Switched to MockLlmProvider (no API calls)");
            continue;
        }
        if input == "/real" {
            if let Some(ref config) = hai_config {
                match make_real_provider(&current_model, config) {
                    Ok(p) => {
                        llm = p;
                        use_mock = false;
                        println!("✅ Switched to real LLM: {}", current_model);
                    }
                    Err(e) => println!("❌ Failed: {}", e),
                }
            } else {
                println!("⚠️  No config loaded.");
            }
            continue;
        }

        // Build conversation and run agentic loop
        let mut messages = vec![
            system_prompt.clone(),
            ChatMessage::user(input),
        ];

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
