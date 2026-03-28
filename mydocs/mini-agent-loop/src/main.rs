
//! # Mini Agent Loop — IronClaw Core Distilled
//!
//! This is a minimal, standalone extraction of IronClaw's agentic loop.
//! It demonstrates the complete message flow:
//!
//! ```text
//! stdin → Agentic Loop → LLM → Tool Execute → Response → stdout
//! ```
//!
//! ## What's included (mapped to IronClaw source files):
//!
//! | This file section       | IronClaw source                          | Purpose                        |
//! |-------------------------|------------------------------------------|--------------------------------|
//! | `LlmProvider` trait     | `src/llm/provider.rs`                    | LLM abstraction                |
//! | `Tool` trait            | `src/tools/tool.rs`                      | Tool abstraction               |
//! | `LoopDelegate` trait    | `src/agent/agentic_loop.rs`              | Strategy pattern for the loop  |
//! | `run_agentic_loop()`    | `src/agent/agentic_loop.rs`              | The core loop engine           |
//! | `execute_tool_with_safety()` | `src/tools/execute.rs`              | Tool execution pipeline        |
//! | `CalculatorTool`        | (like any builtin tool)                  | Example tool                   |
//! | `MockLlmProvider`       | (replaces real OpenAI/Claude provider)   | Simulated LLM for demo         |
//! | `main()`                | (replaces Channel + Agent::run())        | stdin-based entry point        |
//!
//! ## What's intentionally omitted:
//!
//! - Channel layer (replaced by stdin/stdout)
//! - Session/Thread/Turn management (single-turn only)
//! - WASM sandbox (tools are native Rust)
//! - Database persistence
//! - Safety/sanitization layer
//! - Approval flow
//! - Hooks system
//!
//! ## How to run:
//!
//! ```bash
//! cargo run
//! ```
//!
//! Then type messages like:
//! - "What is 42 + 58?"
//! - "Calculate 100 divided by 3"
//! - "Hello, how are you?"
//! - Type "quit" to exit

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::time::Duration;

// ============================================================================
// PART 1: LLM Types & Provider Trait
// ============================================================================
// Corresponds to: src/llm/provider.rs
//
// In IronClaw, LlmProvider is a trait with implementations for OpenAI,
// Anthropic, Mistral, etc. Here we define the minimal types needed.
// ============================================================================

/// Role in a conversation message.
/// Maps to: `src/llm/provider.rs` → `enum Role`
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A message in the conversation context.
/// Maps to: `src/llm/provider.rs` → `struct ChatMessage`
///
/// In IronClaw, ChatMessage also supports multimodal content (images),
/// tool_calls on assistant messages, and tool_call_id on tool results.
/// We keep only what the loop needs.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Tool call ID — set when role == Tool (links result to a specific call)
    pub tool_call_id: Option<String>,
    /// Tool name — set when role == Tool
    pub name: Option<String>,
    /// Tool calls — set when role == Assistant and LLM wants to call tools
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), tool_call_id: None, name: None, tool_calls: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), tool_call_id: None, name: None, tool_calls: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_call_id: None, name: None, tool_calls: None }
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

/// A tool call requested by the LLM.
/// Maps to: `src/llm/provider.rs` → `struct ToolCall`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Definition of a tool for the LLM (sent in the request so LLM knows what's available).
/// Maps to: `src/llm/provider.rs` → `struct ToolDefinition`
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Why the LLM stopped generating.
/// Maps to: `src/llm/provider.rs` → `enum FinishReason`
#[derive(Debug, Clone, PartialEq)]
pub enum FinishReason {
    Stop,     // Normal completion
    ToolUse,  // LLM wants to call a tool
    Length,   // Hit token limit (response truncated)
}

/// Result from the LLM — either text or tool calls.
/// Maps to: `src/llm/reasoning.rs` → `enum RespondResult`
///
/// In IronClaw, this is wrapped in `RespondOutput` which also carries
/// token usage stats. We simplify.
#[derive(Debug)]
pub enum LlmOutput {
    /// LLM returned a text response (conversation continues or ends)
    Text(String),
    /// LLM wants to call one or more tools
    ToolCalls {
        tool_calls: Vec<ToolCall>,
        /// Optional text content alongside tool calls
        content: Option<String>,
    },
}

/// The LLM response including metadata.
#[derive(Debug)]
pub struct LlmResponse {
    pub result: LlmOutput,
    pub finish_reason: FinishReason,
}

/// Trait for LLM providers.
/// Maps to: `src/llm/provider.rs` → `trait LlmProvider`
///
/// In IronClaw, this has `complete()` and `complete_with_tools()` methods,
/// plus cost tracking, model switching, etc. We merge into one method.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Call the LLM with messages and available tools.
    /// Returns either a text response or tool call requests.
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse, String>;
}

// ============================================================================
// PART 2: Tool Trait & Execution Pipeline
// ============================================================================
// Corresponds to: src/tools/tool.rs + src/tools/execute.rs
//
// In IronClaw, Tool is a rich trait with approval, rate limiting, risk levels,
// WASM support, etc. Here we keep only: name, description, schema, execute.
//
// The execution pipeline in IronClaw goes through:
//   lookup → normalize params → validate → timeout → execute → serialize
// We simplify to: lookup → timeout → execute → serialize
// ============================================================================

/// Output from a tool execution.
/// Maps to: `src/tools/tool.rs` → `struct ToolOutput`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub result: serde_json::Value,
}

/// Trait for tools that the agent can use.
/// Maps to: `src/tools/tool.rs` → `trait Tool`
///
/// In IronClaw, this trait has ~15 methods including:
/// - requires_approval() → approval flow
/// - risk_level_for() → risk classification
/// - execution_timeout() → per-tool timeout
/// - domain() → Orchestrator vs Container
/// - sensitive_params() → parameter redaction
/// - rate_limit_config() → rate limiting
/// - webhook_capability() → webhook support
///
/// We keep only the essential 4.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String>;
}

/// Simple tool registry.
/// Maps to: `src/tools/registry.rs` → `struct ToolRegistry`
///
/// In IronClaw, ToolRegistry uses RwLock<HashMap> and supports
/// dynamic registration/deregistration of tools at runtime.
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

/// Execute a tool with timeout.
/// Maps to: `src/tools/execute.rs` → `execute_tool_with_safety()`
///
/// In IronClaw, this function does:
///   1. tools.get(name) — lookup
///   2. prepare_tool_params() — normalize (string arrays → real arrays, etc.)
///   3. safety.validator().validate_tool_params() — injection check
///   4. redact_params() — log with sensitive params hidden
///   5. tokio::time::timeout(tool.execution_timeout(), tool.execute()) — execute
///   6. serde_json::to_string_pretty() — serialize result
///
/// We keep: lookup → timeout → execute → serialize
async fn execute_tool_with_safety(
    registry: &ToolRegistry,
    tool_name: &str,
    params: serde_json::Value,
) -> Result<String, String> {
    // Step 1: Lookup
    let tool = registry.get(tool_name)
        .ok_or_else(|| format!("Tool '{}' not found", tool_name))?;

    println!("  ⚙️  Executing tool: {} with params: {}", tool_name, params);

    // Step 2: Execute with timeout (IronClaw uses per-tool timeout, default 60s)
    let timeout = Duration::from_secs(30);
    let result = tokio::time::timeout(timeout, tool.execute(params)).await;

    match result {
        Ok(Ok(output)) => {
            // Step 3: Serialize result
            serde_json::to_string_pretty(&output.result)
                .map_err(|e| format!("Failed to serialize result: {}", e))
        }
        Ok(Err(e)) => Err(format!("Tool execution failed: {}", e)),
        Err(_) => Err(format!("Tool '{}' timed out after {:?}", tool_name, timeout)),
    }
}

/// Process a tool result into a ChatMessage.
/// Maps to: `src/tools/execute.rs` → `process_tool_result()`
///
/// In IronClaw, this also:
///   1. safety.sanitize_tool_output() — remove sensitive data
///   2. safety.wrap_for_llm() — wrap in <tool_output> XML tags
/// We skip sanitization and wrapping for simplicity.
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
// Corresponds to: src/agent/agentic_loop.rs
//
// This is the HEART of the system. The loop:
//   1. Calls LLM with conversation context + available tools
//   2. If LLM returns text → done (or continue if delegate says so)
//   3. If LLM returns tool calls → execute tools → add results to context → goto 1
//   4. Repeat until text response or max iterations
//
// In IronClaw, the loop is generic via LoopDelegate trait, serving:
//   - ChatDelegate (interactive chat)
//   - JobDelegate (background jobs)
//   - ContainerDelegate (Docker containers)
//
// Here we inline a simplified version without the delegate pattern.
// ============================================================================

/// Configuration for the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `struct AgenticLoopConfig`
pub struct AgenticLoopConfig {
    pub max_iterations: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self { max_iterations: 10 }  // IronClaw default is 50
    }
}

/// Outcome of the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `enum LoopOutcome`
///
/// In IronClaw, this also has:
///   - Stopped (external signal)
///   - NeedApproval (tool requires user confirmation)
pub enum LoopOutcome {
    Response(String),
    MaxIterations,
}

/// Run the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `run_agentic_loop()`
///
/// This is the single most important function in the entire system.
///
/// In IronClaw, this function:
///   1. check_signals() — check for cancellation/stop/inject
///   2. before_llm_call() — refresh tools, cost guard, inject context
///   3. call_llm() — call the LLM provider
///   4. Handle text → delegate.handle_text_response()
///   5. Handle tool calls → delegate.execute_tool_calls()
///   6. Tool intent nudge — if LLM says "let me search" without calling a tool
///   7. Truncation handling — if response was cut off (finish_reason=Length)
///   8. after_iteration() — post-iteration hook
///
/// We keep: call LLM → handle text/tools → loop
async fn run_agentic_loop(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = registry.definitions();

    for iteration in 1..=config.max_iterations {
        println!("\n--- Iteration {}/{} ---", iteration, config.max_iterations);

        // ── Step 1: Call LLM ──
        // In IronClaw: delegate.call_llm(reasoning, reason_ctx, iteration)
        // The delegate handles rate limiting, auto-compaction, cost tracking,
        // force_text mode, and model selection.
        let response = llm.chat(messages, &tool_defs).await?;

        match response.result {
            // ── Step 2a: Text Response ──
            // In IronClaw: delegate.handle_text_response() returns TextAction::Return or Continue
            // ChatDelegate returns Return (end loop), JobDelegate may return Continue
            // (if it detects the job isn't done yet).
            LlmOutput::Text(text) => {
                println!("  💬 LLM returned text: {}", &text[..text.len().min(100)]);
                return Ok(LoopOutcome::Response(text));
            }

            // ── Step 2b: Tool Calls ──
            // In IronClaw: delegate.execute_tool_calls() handles:
            //   - Truncation check (finish_reason == Length → discard malformed calls)
            //   - Approval check (requires_approval → pause loop)
            //   - Parallel execution (JoinSet for multiple tools)
            //   - Status updates (ToolStarted, ToolCompleted events)
            //   - Safety sanitization of results
            //   - Hook dispatch (BeforeToolCall, AfterToolCall)
            LlmOutput::ToolCalls { tool_calls, content } => {
                println!("  🔧 LLM wants to call {} tool(s)", tool_calls.len());

                // In IronClaw: if finish_reason == Length, discard truncated tool calls
                // and inject a notice telling LLM to try a different approach.
                if response.finish_reason == FinishReason::Length {
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        messages.push(ChatMessage::assistant(&text));
                    }
                    messages.push(ChatMessage::user(
                        "Your previous response was truncated. Please try a simpler approach."
                    ));
                    continue;
                }

                // Add assistant message with tool calls to context
                // (OpenAI protocol requires this before tool results)
                messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                // Execute each tool and add results to context
                for tc in &tool_calls {
                    let result = execute_tool_with_safety(registry, &tc.name, tc.arguments.clone()).await;

                    // In IronClaw: process_tool_result() sanitizes output and wraps in XML
                    let result_msg = process_tool_result(&tc.name, &tc.id, &result);

                    match &result {
                        Ok(output) => println!("  ✅ Tool '{}' succeeded: {}",
                            tc.name, &output[..output.len().min(80)]),
                        Err(e) => println!("  ❌ Tool '{}' failed: {}", tc.name, e),
                    }

                    messages.push(result_msg);
                }

                // Loop continues — LLM will see tool results in next iteration
            }
        }
    }

    Ok(LoopOutcome::MaxIterations)
}

// ============================================================================
// PART 4: Example Tool — Calculator
// ============================================================================
// In IronClaw, tools can be:
//   - Builtin (Rust native): shell, http, file_read, file_write, memory, etc.
//   - WASM (sandboxed): third-party tools running in Wasmtime sandbox
//   - MCP (external): tools accessed via Model Context Protocol
//
// The Calculator here is a simple builtin tool for demonstration.
// ============================================================================

struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str { "calculator" }

    fn description(&self) -> &str {
        "Perform basic arithmetic operations. Supports: add, sub, mul, div."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "sub", "mul", "div"],
                    "description": "The arithmetic operation to perform"
                },
                "a": { "type": "number", "description": "First operand" },
                "b": { "type": "number", "description": "Second operand" }
            },
            "required": ["operation", "a", "b"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let op = params["operation"].as_str().ok_or("Missing 'operation'")?;
        let a = params["a"].as_f64().ok_or("Missing 'a'")?;
        let b = params["b"].as_f64().ok_or("Missing 'b'")?;

        let (result, expression) = match op {
            "add" => (a + b, format!("{} + {} = {}", a, b, a + b)),
            "sub" => (a - b, format!("{} - {} = {}", a, b, a - b)),
            "mul" => (a * b, format!("{} × {} = {}", a, b, a * b)),
            "div" => {
                if b == 0.0 {
                    return Err("Division by zero".to_string());
                }
                (a / b, format!("{} ÷ {} = {}", a, b, a / b))
            }
            _ => return Err(format!("Unknown operation: '{}'", op)),
        };

        Ok(ToolOutput {
            result: serde_json::json!({
                "result": result,
                "expression": expression,
            }),
        })
    }
}

// ============================================================================
// PART 5: Mock LLM Provider
// ============================================================================
// In IronClaw, real providers (OpenAI, Anthropic, etc.) make HTTP calls.
// This mock simulates LLM behavior by pattern-matching on user input:
//   - Math questions → returns tool call to calculator
//   - After receiving tool result → returns text summary
//   - Everything else → returns direct text response
//
// This lets you run the demo without an API key while seeing the full
// agentic loop in action.
// ============================================================================

struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolDefinition],
    ) -> Result<LlmResponse, String> {
        // Find the last user message
        let last_user = messages.iter().rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");

        // Check if we just received a tool result (last message is a tool result)
        let last_msg = messages.last();
        if let Some(msg) = last_msg {
            if msg.role == Role::Tool {
                // We have a tool result — summarize it as text
                // In a real LLM, it would read the tool output and formulate a response
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

        // Pattern match on user input to decide: text response or tool call?
        let lower = last_user.to_lowercase();
        let is_math = lower.contains('+') || lower.contains('-') || lower.contains('*')
            || lower.contains('/') || lower.contains("plus") || lower.contains("minus")
            || lower.contains("times") || lower.contains("multiply")
            || lower.contains("divide") || lower.contains("divided")
            || lower.contains("add") || lower.contains("subtract")
            || lower.contains("calculate") || lower.contains("what is")
            || lower.contains("how much");

        if is_math {
            // Parse numbers and operation from the input
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
            // Direct text response (no tool needed)
            Ok(LlmResponse {
                result: LlmOutput::Text(format!(
                    "I'm a mini agent demo. I can do math! Try asking me something like \
                     'What is 42 + 58?' or 'Calculate 100 divided by 3'. \
                     You said: \"{}\"",
                    last_user
                )),
                finish_reason: FinishReason::Stop,
            })
        }
    }
}

/// Simple math intent parser for the mock LLM.
/// A real LLM would understand natural language; this is just pattern matching.
fn parse_math_intent(input: &str) -> (&str, f64, f64) {
    let lower = input.to_lowercase();

    // Try to find two numbers in the input
    let numbers: Vec<f64> = input
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    let a = numbers.first().copied().unwrap_or(0.0);
    let b = numbers.get(1).copied().unwrap_or(0.0);

    // Determine operation
    let op = if lower.contains('+') || lower.contains("plus") || lower.contains("add") {
        "add"
    } else if lower.contains('-') || lower.contains("minus") || lower.contains("subtract") {
        "sub"
    } else if lower.contains('*') || lower.contains("times") || lower.contains("multiply") {
        "mul"
    } else if lower.contains('/') || lower.contains("divide") {
        "div"
    } else {
        "add" // default
    };

    (op, a, b)
}

// ============================================================================
// PART 6: Main — The Entry Point (replaces Channel + Agent::run())
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
    println!("║  Try:                                                      ║");
    println!("║    • \"What is 42 + 58?\"                                    ║");
    println!("║    • \"Calculate 100 divided by 3\"                          ║");
    println!("║    • \"Hello\" (direct response, no tool)                    ║");
    println!("║    • \"quit\" to exit                                        ║");
    println!("║                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // ── Setup (corresponds to Agent::new() in IronClaw) ──

    // 1. Create LLM provider
    //    In IronClaw: configured via config.toml, supports OpenAI/Anthropic/Mistral/etc.
    let llm: Box<dyn LlmProvider> = Box::new(MockLlmProvider);

    // 2. Create tool registry and register tools
    //    In IronClaw: tools are registered from config, including WASM tools loaded from disk
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CalculatorTool));

    // 3. Create system prompt
    //    In IronClaw: built from agent persona + skill context + tool list + conversation context
    let system_prompt = ChatMessage::system(
        "You are a helpful assistant with access to a calculator tool. \
         When the user asks a math question, use the calculator tool to compute the answer. \
         For non-math questions, respond directly."
    );

    // 4. Loop config
    let config = AgenticLoopConfig::default();

    // ── Main Loop (corresponds to Agent::run() event loop) ──
    //
    // In IronClaw:
    //   loop {
    //       let message = tokio::select! {
    //           _ = ctrl_c() => break,
    //           msg = message_stream.next() => msg,
    //       };
    //       // preprocess (transcription, doc extraction)
    //       let response = self.handle_message(&message).await;
    //   }
    //
    // We simplify to a stdin loop:

    loop {
        // ── Read input (replaces Channel::start() → MessageStream) ──
        print!("\n🧑 You: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim();

        if input.is_empty() {
            continue;
        }
        if input == "quit" || input == "exit" {
            println!("👋 Goodbye!");
            break;
        }

        // ── Build conversation context ──
        // In IronClaw: session.thread.messages() collects all historical messages
        // including system prompt, past turns, tool calls and results.
        // Here we start fresh each turn (no history).
        let mut messages = vec![
            system_prompt.clone(),
            ChatMessage::user(input),
        ];

        // ── Run the agentic loop ──
        // In IronClaw: self.run_agentic_loop(message, tenant, session, thread_id, messages)
        // This is where the magic happens!
        println!("\n🤖 Agent thinking...");
        match run_agentic_loop(llm.as_ref(), &registry, &mut messages, &config).await {
            Ok(LoopOutcome::Response(text)) => {
                // In IronClaw: thread.complete_turn(&response) + persist to DB
                // + hooks.run(ResponseTransform) + channel.respond()
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
