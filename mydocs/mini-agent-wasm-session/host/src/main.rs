//! # Mini Agent Loop + WASM Sandbox + Session — IronClaw Core Distilled
//!
//! Builds on mini-agent-wasm by adding the Session/Thread/Turn model.
//! This is the conversation management layer that IronClaw uses to track
//! multi-turn interactions.
//!
//! ```text
//! Session (per user)
//! └── Thread (per conversation — can have many)
//!     └── Turn (per request/response pair)
//!         ├── user_input: String
//!         ├── response: Option<String>
//!         ├── tool_calls: Vec<TurnToolCall>
//!         └── state: TurnState
//! ```
//!
//! ## What's new compared to mini-agent-wasm:
//!
//! | mini-agent-wasm                | mini-agent-wasm-session                  |
//! |-------------------------------|------------------------------------------|
//! | Single-turn (no history)       | Multi-turn with full conversation memory |
//! | No session concept             | Session → Thread → Turn hierarchy        |
//! | Fresh messages each input      | Messages rebuilt from Turn history        |
//! | No thread management           | /new, /threads, /switch, /history        |
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        Host Process                                 │
//! │                                                                     │
//! │  ┌──────────┐    ┌──────────────────────────────────────────────┐  │
//! │  │  stdin    │───▶│  Session Manager                             │  │
//! │  │  stdout   │◀──│    └── Session (user: "cli")                 │  │
//! │  └──────────┘    │        ├── Thread #1 (active)                │  │
//! │                  │        │   ├── Turn 1: "Hi" → "Hello!"       │  │
//! │                  │        │   ├── Turn 2: "42+58?" → [calc] 100 │  │
//! │                  │        │   └── Turn 3: (in progress...)      │  │
//! │                  │        └── Thread #2                          │  │
//! │                  └──────────────┬───────────────────────────────┘  │
//! │                                 │                                   │
//! │                                 │ messages from Turn history         │
//! │                                 ▼                                   │
//! │  ┌──────────────────────────────────────────────────────────────┐  │
//! │  │  Agentic Loop                                                │  │
//! │  │    LLM ←→ Tool Execution ←→ WASM Sandbox                    │  │
//! │  └──────────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Maps to IronClaw:
//!   src/agent/session.rs         → Session, Thread, Turn, TurnState
//!   src/agent/session_manager.rs → SessionManager
//!   src/agent/thread_ops.rs      → process_user_input()

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use wasmtime::component::Linker;
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

// ============================================================================
// PART 1: LLM Types & Provider Trait
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
// PART 2: Session / Thread / Turn Model
// ============================================================================
// THIS IS THE KEY ADDITION compared to mini-agent-wasm.
//
// Maps to IronClaw's src/agent/session.rs:
//   Session → Thread → Turn hierarchy
//   Thread::messages() rebuilds ChatMessage list from Turn history
//   TurnToolCall records each tool invocation within a turn
//
// The key insight: the LLM needs the FULL conversation history as context.
// Instead of building messages from scratch each time (mini-agent-wasm),
// we store structured Turns and rebuild messages from them.
// ============================================================================

/// State of a turn.
///
/// Maps to: `src/agent/session.rs` → `TurnState`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    /// Turn is being processed by the agent.
    Processing,
    /// Turn completed successfully with a response.
    Completed,
    /// Turn failed with an error.
    Failed,
}

/// A tool call recorded within a turn.
///
/// Maps to: `src/agent/session.rs` → `TurnToolCall`
#[derive(Debug, Clone)]
pub struct TurnToolCall {
    /// Tool name.
    pub name: String,
    /// Parameters passed to the tool (JSON).
    pub parameters: serde_json::Value,
    /// Result from the tool (JSON), if completed.
    pub result: Option<serde_json::Value>,
    /// Error message, if the tool failed.
    pub error: Option<String>,
}

/// A single turn (request/response pair) in a thread.
///
/// Maps to: `src/agent/session.rs` → `Turn`
///
/// A turn represents one user input and the agent's complete response,
/// including any tool calls made along the way.
#[derive(Debug, Clone)]
pub struct Turn {
    /// Turn number (0-indexed).
    pub turn_number: usize,
    /// User input that started this turn.
    pub user_input: String,
    /// Agent response (if completed).
    pub response: Option<String>,
    /// Tool calls made during this turn.
    pub tool_calls: Vec<TurnToolCall>,
    /// Turn state.
    pub state: TurnState,
    /// When the turn started.
    pub started_at: DateTime<Utc>,
    /// When the turn completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message (if failed).
    pub error: Option<String>,
}

impl Turn {
    /// Create a new turn.
    pub fn new(turn_number: usize, user_input: impl Into<String>) -> Self {
        Self {
            turn_number,
            user_input: user_input.into(),
            response: None,
            tool_calls: Vec::new(),
            state: TurnState::Processing,
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    /// Record a tool call in this turn.
    pub fn record_tool_call(&mut self, name: &str, parameters: serde_json::Value) {
        self.tool_calls.push(TurnToolCall {
            name: name.to_string(),
            parameters,
            result: None,
            error: None,
        });
    }

    /// Record a tool result for the last tool call.
    pub fn record_tool_result(&mut self, result: serde_json::Value) {
        if let Some(tc) = self.tool_calls.last_mut() {
            tc.result = Some(result);
        }
    }

    /// Record a tool error for the last tool call.
    pub fn record_tool_error(&mut self, error: String) {
        if let Some(tc) = self.tool_calls.last_mut() {
            tc.error = Some(error);
        }
    }

    /// Complete the turn with a response.
    pub fn complete(&mut self, response: impl Into<String>) {
        self.response = Some(response.into());
        self.state = TurnState::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Fail the turn with an error.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.state = TurnState::Failed;
        self.completed_at = Some(Utc::now());
    }
}

/// State of a thread.
///
/// Maps to: `src/agent/session.rs` → `ThreadState`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Thread is idle, waiting for input.
    Idle,
    /// Thread is processing a turn.
    Processing,
}

/// A conversation thread containing turns.
///
/// Maps to: `src/agent/session.rs` → `Thread`
///
/// The thread is the core unit of conversation. It maintains an ordered
/// list of turns and can rebuild the full ChatMessage history from them.
#[derive(Debug, Clone)]
pub struct Thread {
    /// Unique thread ID.
    pub id: Uuid,
    /// Parent session ID.
    pub session_id: Uuid,
    /// Current state.
    pub state: ThreadState,
    /// Turns in this thread.
    pub turns: Vec<Turn>,
    /// When the thread was created.
    pub created_at: DateTime<Utc>,
    /// When the thread was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Thread {
    /// Create a new thread.
    pub fn new(session_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Start a new turn with user input.
    pub fn start_turn(&mut self, user_input: impl Into<String>) -> &mut Turn {
        let turn_number = self.turns.len();
        let turn = Turn::new(turn_number, user_input);
        self.turns.push(turn);
        self.state = ThreadState::Processing;
        self.updated_at = Utc::now();
        &mut self.turns[turn_number]
    }

    /// Complete the current turn with a response.
    pub fn complete_turn(&mut self, response: impl Into<String>) {
        if let Some(turn) = self.turns.last_mut() {
            turn.complete(response);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Fail the current turn with an error.
    pub fn fail_turn(&mut self, error: impl Into<String>) {
        if let Some(turn) = self.turns.last_mut() {
            turn.fail(error);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Get the last turn.
    pub fn last_turn(&self) -> Option<&Turn> {
        self.turns.last()
    }

    /// Get the last turn mutably.
    pub fn last_turn_mut(&mut self) -> Option<&mut Turn> {
        self.turns.last_mut()
    }

    /// Rebuild ChatMessage history from turns.
    ///
    /// This is THE KEY METHOD — it converts the structured Turn history
    /// into the flat ChatMessage list that the LLM expects.
    ///
    /// Maps to: `src/agent/session.rs` → `Thread::messages()`
    ///
    /// For each turn, it produces:
    ///   1. User message (the input)
    ///   2. If tool calls: assistant message declaring tool calls + tool result messages
    ///   3. If response: assistant message with the final text
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        for (turn_idx, turn) in self.turns.iter().enumerate() {
            // 1. User message
            messages.push(ChatMessage::user(&turn.user_input));

            // 2. Tool calls (if any)
            if !turn.tool_calls.is_empty() {
                // Generate synthetic tool call IDs (deterministic from turn/tool indices)
                let tool_calls_with_ids: Vec<(String, &TurnToolCall)> = turn
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(tc_idx, tc)| {
                        (format!("call_{turn_idx}_{tc_idx}"), tc)
                    })
                    .collect();

                // Assistant message declaring the tool calls
                let tool_calls: Vec<ToolCall> = tool_calls_with_ids
                    .iter()
                    .map(|(call_id, tc)| ToolCall {
                        id: call_id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.parameters.clone(),
                    })
                    .collect();
                messages.push(ChatMessage::assistant_with_tool_calls(None, tool_calls));

                // Tool result messages
                for (call_id, tc) in &tool_calls_with_ids {
                    let content = if let Some(ref err) = tc.error {
                        format!("Error: {}", err)
                    } else if let Some(ref res) = tc.result {
                        match res {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        }
                    } else {
                        "OK".to_string()
                    };
                    messages.push(ChatMessage::tool_result(call_id, &tc.name, content));
                }
            }

            // 3. Assistant response (if completed)
            if let Some(ref response) = turn.response {
                messages.push(ChatMessage::assistant(response));
            }
        }

        messages
    }
}

/// A session containing one or more threads.
///
/// Maps to: `src/agent/session.rs` → `Session`
///
/// In IronClaw, a session is per-user. Here we have a single CLI user,
/// but the structure supports multiple threads within the session.
#[derive(Debug)]
pub struct Session {
    /// Unique session ID.
    pub id: Uuid,
    /// User ID that owns this session.
    pub user_id: String,
    /// Active thread ID.
    pub active_thread: Option<Uuid>,
    /// All threads in this session.
    pub threads: HashMap<Uuid, Thread>,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last active.
    pub last_active_at: DateTime<Utc>,
}

impl Session {
    /// Create a new session.
    pub fn new(user_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: user_id.into(),
            active_thread: None,
            threads: HashMap::new(),
            created_at: now,
            last_active_at: now,
        }
    }

    /// Create a new thread in this session.
    pub fn create_thread(&mut self) -> &mut Thread {
        let thread = Thread::new(self.id);
        let thread_id = thread.id;
        self.active_thread = Some(thread_id);
        self.last_active_at = Utc::now();
        self.threads.entry(thread_id).or_insert(thread)
    }

    /// Get the active thread.
    pub fn active_thread(&self) -> Option<&Thread> {
        self.active_thread.and_then(|id| self.threads.get(&id))
    }

    /// Get the active thread mutably.
    pub fn active_thread_mut(&mut self) -> Option<&mut Thread> {
        self.active_thread.and_then(|id| self.threads.get_mut(&id))
    }

    /// Get or create the active thread.
    pub fn get_or_create_thread(&mut self) -> &mut Thread {
        match self.active_thread {
            Some(id) if self.threads.contains_key(&id) => {
                self.threads.get_mut(&id).unwrap()
            }
            _ => self.create_thread(),
        }
    }

    /// Switch to a different thread.
    pub fn switch_thread(&mut self, thread_id: Uuid) -> bool {
        if self.threads.contains_key(&thread_id) {
            self.active_thread = Some(thread_id);
            self.last_active_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// List all threads with summary info.
    pub fn list_threads(&self) -> Vec<ThreadSummary> {
        let mut summaries: Vec<_> = self.threads.values().map(|t| {
            let first_input = t.turns.first().map(|turn| {
                let s = &turn.user_input;
                if s.len() > 40 { format!("{}...", &s[..40]) } else { s.clone() }
            });
            ThreadSummary {
                id: t.id,
                is_active: self.active_thread == Some(t.id),
                turn_count: t.turns.len(),
                state: t.state,
                created_at: t.created_at,
                first_input,
            }
        }).collect();
        summaries.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        summaries
    }
}

/// Summary info for a thread (used in /threads listing).
pub struct ThreadSummary {
    pub id: Uuid,
    pub is_active: bool,
    pub turn_count: usize,
    pub state: ThreadState,
    pub created_at: DateTime<Utc>,
    pub first_input: Option<String>,
}

// ============================================================================
// PART 3: Tool Trait & WASM Sandbox Execution
// ============================================================================
// Same as mini-agent-wasm — unchanged.
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub result: serde_json::Value,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String>;
}

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
// WASM Sandbox Infrastructure (same as mini-agent-wasm)
// ════════════════════════════════════════════════════════════════════

wasmtime::component::bindgen!({
    path: "../wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});

struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    logs: Vec<(String, String)>,
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> &mut WasiCtx { &mut self.wasi }
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
}

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

struct WasmToolEngine {
    engine: Engine,
    component: wasmtime::component::Component,
    tool_name: String,
    tool_description: String,
    tool_schema: serde_json::Value,
    fuel_limit: u64,
}

impl WasmToolEngine {
    fn new(wasm_path: &str, tool_name: &str, fuel_limit: u64) -> Result<Self, String> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|e| format!("Failed to create WASM engine: {}", e))?;

        let wasm_bytes = std::fs::read(wasm_path)
            .map_err(|e| format!("Failed to read WASM file '{}': {}", wasm_path, e))?;
        let component = wasmtime::component::Component::new(&engine, &wasm_bytes)
            .map_err(|e| format!("Failed to compile WASM component: {}", e))?;

        let mut linker: Linker<StoreData> = Linker::new(&engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("Failed to add WASI to linker: {}", e))?;
        demo::sandbox::host::add_to_linker(&mut linker, |state| state)
            .map_err(|e| format!("Failed to add host functions to linker: {}", e))?;

        let mut store = Store::new(&engine, StoreData {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            logs: Vec::new(),
        });
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
            engine, component,
            tool_name: tool_name.to_string(),
            tool_description: description,
            tool_schema: schema,
            fuel_limit,
        })
    }

    fn execute_in_sandbox(&self, params: &str) -> Result<String, String> {
        let mut linker: Linker<StoreData> = Linker::new(&self.engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("WASI linker error: {}", e))?;
        demo::sandbox::host::add_to_linker(&mut linker, |state| state)
            .map_err(|e| format!("Host linker error: {}", e))?;

        let mut store = Store::new(&self.engine, StoreData {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            logs: Vec::new(),
        });
        store.set_fuel(self.fuel_limit).map_err(|e| format!("Fuel error: {}", e))?;

        let instance = SandboxedTool::instantiate(&mut store, &self.component, &linker)
            .map_err(|e| format!("Instantiation error: {}", e))?;

        let request = exports::demo::sandbox::tool::Request {
            params: params.to_string(),
        };
        let response = instance.demo_sandbox_tool().call_execute(&mut store, &request)
            .map_err(|e| format!("WASM execution error: {}", e))?;

        let remaining = store.get_fuel().unwrap_or(0);
        let consumed = self.fuel_limit - remaining;
        println!("    ⛽ Fuel consumed: {} / {} units", consumed, self.fuel_limit);

        if let Some(error) = response.error {
            return Err(format!("Tool error: {}", error));
        }
        response.output.ok_or_else(|| "Tool returned no output".to_string())
    }
}

struct WasmTool {
    engine: Arc<WasmToolEngine>,
}

#[async_trait]
impl Tool for WasmTool {
    fn name(&self) -> &str { &self.engine.tool_name }
    fn description(&self) -> &str { &self.engine.tool_description }
    fn parameters_schema(&self) -> serde_json::Value { self.engine.tool_schema.clone() }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let params_str = serde_json::to_string(&params)
            .map_err(|e| format!("Failed to serialize params: {}", e))?;
        let engine = self.engine.clone();
        let result = tokio::task::spawn_blocking(move || {
            engine.execute_in_sandbox(&params_str)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))??;

        let value: serde_json::Value = serde_json::from_str(&result)
            .map_err(|e| format!("Failed to parse tool output: {}", e))?;
        Ok(ToolOutput { result: value })
    }
}

// ============================================================================
// PART 4: Tool Execution Pipeline (with Turn recording)
// ============================================================================
// Key difference from mini-agent-wasm: tool calls and results are recorded
// into the current Turn, not just appended to a flat message list.
// ============================================================================

/// Execute a tool with timeout.
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
// PART 5: Agentic Loop (with Turn integration)
// ============================================================================
// Key difference: the loop now records tool calls into the current Turn,
// and the final response is returned for the caller to record.
// ============================================================================

pub struct AgenticLoopConfig {
    pub max_iterations: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self { max_iterations: 10 }
    }
}

/// Outcome of the agentic loop — includes tool call info for Turn recording.
pub enum LoopOutcome {
    /// LLM returned a final text response.
    Response {
        text: String,
        /// Tool calls made during this turn (name, params, result/error).
        tool_calls: Vec<(String, serde_json::Value, Result<String, String>)>,
    },
    /// Reached max iterations without a final response.
    MaxIterations {
        tool_calls: Vec<(String, serde_json::Value, Result<String, String>)>,
    },
}

/// Run the agentic loop.
///
/// Key difference from mini-agent-wasm: we track tool calls made during
/// the loop so they can be recorded into the Turn afterwards.
async fn run_agentic_loop(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = registry.definitions();
    let mut recorded_tool_calls: Vec<(String, serde_json::Value, Result<String, String>)> = Vec::new();

    for iteration in 1..=config.max_iterations {
        println!("\n--- Iteration {}/{} ---", iteration, config.max_iterations);

        let response = llm.chat(messages, &tool_defs).await?;

        match response.result {
            LlmOutput::Text(text) => {
                println!("  💬 LLM returned text: {}", &text[..text.len().min(100)]);
                return Ok(LoopOutcome::Response {
                    text,
                    tool_calls: recorded_tool_calls,
                });
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

                    // Record for Turn
                    recorded_tool_calls.push((tc.name.clone(), tc.arguments.clone(), result));

                    messages.push(result_msg);
                }
            }
        }
    }

    Ok(LoopOutcome::MaxIterations {
        tool_calls: recorded_tool_calls,
    })
}

// ============================================================================
// PART 6: Process User Input (Session-aware)
// ============================================================================
// This is the new orchestration layer that ties Session + Agentic Loop together.
//
// Maps to: `src/agent/thread_ops.rs` → `process_user_input()`
//
// Flow:
//   1. Get or create the active thread
//   2. Start a new Turn in the thread
//   3. Rebuild messages from Turn history (Thread::messages())
//   4. Prepend system prompt
//   5. Run the agentic loop
//   6. Record tool calls and response into the Turn
//   7. Complete or fail the Turn
// ============================================================================

async fn process_user_input(
    session: &mut Session,
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    system_prompt: &ChatMessage,
    loop_config: &AgenticLoopConfig,
    user_input: &str,
) -> Result<String, String> {
    // 1. Get or create the active thread
    let thread = session.get_or_create_thread();
    let thread_id = thread.id;

    // 2. Start a new Turn
    thread.start_turn(user_input);
    println!("  📝 Turn #{} started in thread {}", thread.turns.len(), &thread_id.to_string()[..8]);

    // 3. Rebuild messages from Turn history
    //    This is THE KEY: instead of starting fresh, we get the full
    //    conversation history from all previous turns.
    let mut messages = vec![system_prompt.clone()];
    messages.extend(thread.messages());

    println!("  📨 Context: {} messages (from {} turns)",
        messages.len(), thread.turns.len());

    // 4. Run the agentic loop
    let outcome = run_agentic_loop(llm, registry, &mut messages, loop_config).await;

    // 5. Record results into the Turn
    let thread = session.threads.get_mut(&thread_id).unwrap();

    match outcome {
        Ok(LoopOutcome::Response { text, tool_calls }) => {
            // Record tool calls into the turn
            if let Some(turn) = thread.last_turn_mut() {
                for (name, params, result) in &tool_calls {
                    turn.record_tool_call(name, params.clone());
                    match result {
                        Ok(output) => turn.record_tool_result(
                            serde_json::Value::String(output.clone())
                        ),
                        Err(e) => turn.record_tool_error(e.clone()),
                    }
                }
            }
            // Complete the turn
            thread.complete_turn(&text);
            Ok(text)
        }
        Ok(LoopOutcome::MaxIterations { tool_calls }) => {
            if let Some(turn) = thread.last_turn_mut() {
                for (name, params, result) in &tool_calls {
                    turn.record_tool_call(name, params.clone());
                    match result {
                        Ok(output) => turn.record_tool_result(
                            serde_json::Value::String(output.clone())
                        ),
                        Err(e) => turn.record_tool_error(e.clone()),
                    }
                }
            }
            thread.fail_turn("Reached maximum iterations");
            Err("Reached maximum iterations without a final response.".to_string())
        }
        Err(e) => {
            thread.fail_turn(&e);
            Err(e)
        }
    }
}

// ============================================================================
// PART 7: Mock LLM Provider
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
            // Count conversation history to show multi-turn awareness
            let turn_count = messages.iter().filter(|m| m.role == Role::User).count();
            Ok(LlmResponse {
                result: LlmOutput::Text(format!(
                    "I'm a mini agent with WASM sandbox + session management! \
                     This is turn #{} in our conversation. I can do math! \
                     Try: 'What is 42 + 58?' You said: \"{}\"",
                    turn_count, last_user
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
// PART 8: Real OpenAI-Compatible LLM Provider
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
// PART 9: Main — Entry Point with Session Management
// ============================================================================
// Key differences from mini-agent-wasm:
//   1. Creates a Session at startup
//   2. Each user input goes through process_user_input() which manages Turns
//   3. Supports thread management commands (/new, /threads, /switch, /history)
//   4. Multi-turn: LLM sees full conversation history from previous turns
// ============================================================================

#[tokio::main]
async fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║   Mini Agent + WASM Sandbox + Session — IronClaw Distilled ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║  NEW: Session/Thread/Turn conversation management!         ║");
    println!("║    • Multi-turn: LLM remembers your conversation history   ║");
    println!("║    • Multiple threads: start new conversations with /new   ║");
    println!("║    • Turn tracking: each Q&A pair is a recorded Turn       ║");
    println!("║                                                            ║");
    println!("║  Session commands:                                         ║");
    println!("║    /new              Create a new conversation thread      ║");
    println!("║    /threads          List all threads                      ║");
    println!("║    /switch <n>       Switch to thread #n                   ║");
    println!("║    /history          Show turn history                     ║");
    println!("║    /session          Show session info                     ║");
    println!("║                                                            ║");
    println!("║  Other commands:                                           ║");
    println!("║    /model <name>     Switch LLM model                      ║");
    println!("║    /models           List available models                  ║");
    println!("║    /mock             Switch to mock LLM                    ║");
    println!("║    quit              Exit                                   ║");
    println!("║                                                            ║");
    println!("║  Try multi-turn:                                           ║");
    println!("║    1. \"What is 42 + 58?\"                                   ║");
    println!("║    2. \"Now multiply that result by 3\"                      ║");
    println!("║    3. \"/history\" to see the conversation                   ║");
    println!("║                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // ── Step 1: Load WASM tool ──
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
    let registry = {
        let mut r = ToolRegistry::new();
        r.register(Box::new(WasmTool {
            engine: Arc::new(wasm_engine),
        }));
        r
    };

    println!("✅ Tool registry ready (WASM sandboxed tools)");
    for def in registry.definitions() {
        println!("   📦 {} — {}", def.name, def.description);
    }
    println!();

    // ── Step 4: Create Session ──
    // In IronClaw: SessionManager creates sessions per user
    // Here: single CLI user, one session
    let mut session = Session::new("cli-user");
    session.create_thread(); // Start with one thread
    println!("✅ Session created: {}", &session.id.to_string()[..8]);
    println!("   Thread: {}", &session.active_thread.unwrap().to_string()[..8]);
    println!();

    let system_prompt = ChatMessage::system(
        "You are a helpful assistant with access to a calculator tool. \
         When the user asks a math question, use the calculator tool to compute the answer. \
         For non-math questions, respond directly. \
         You have conversation memory — you can reference previous turns in this thread."
    );

    let loop_config = AgenticLoopConfig::default();

    // ── Step 5: Main loop ──
    loop {
        // Show thread info in prompt
        let thread_info = session.active_thread()
            .map(|t| format!("T{}:{}", t.turns.len() + 1, &t.id.to_string()[..4]))
            .unwrap_or_else(|| "?".to_string());

        print!("\n🧑 [{}] You: ", thread_info);
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
            println!("👋 Goodbye! Session had {} thread(s), {} total turn(s).",
                session.threads.len(),
                session.threads.values().map(|t| t.turns.len()).sum::<usize>());
            break;
        }

        // ── Session management commands ──

        if input == "/new" {
            let thread = session.create_thread();
            println!("✅ New thread created: {}", &thread.id.to_string()[..8]);
            println!("   Now in thread {} (0 turns)", &thread.id.to_string()[..8]);
            continue;
        }

        if input == "/threads" {
            let summaries = session.list_threads();
            println!("\n📋 Threads ({}):", summaries.len());
            for (i, s) in summaries.iter().enumerate() {
                let active = if s.is_active { " ← active" } else { "" };
                let preview = s.first_input.as_deref().unwrap_or("(empty)");
                println!("   #{} [{}] {} turn(s) — \"{}\"{}", 
                    i + 1,
                    &s.id.to_string()[..8],
                    s.turn_count,
                    preview,
                    active);
            }
            continue;
        }

        if input.starts_with("/switch ") {
            let idx_str = input.strip_prefix("/switch ").unwrap().trim();
            if let Ok(idx) = idx_str.parse::<usize>() {
                let summaries = session.list_threads();
                if idx >= 1 && idx <= summaries.len() {
                    let thread_id = summaries[idx - 1].id;
                    if session.switch_thread(thread_id) {
                        let thread = session.active_thread().unwrap();
                        println!("✅ Switched to thread #{} [{}] ({} turns)",
                            idx, &thread_id.to_string()[..8], thread.turns.len());
                    } else {
                        println!("❌ Failed to switch thread");
                    }
                } else {
                    println!("❌ Invalid thread number. Use /threads to see available threads.");
                }
            } else {
                println!("❌ Usage: /switch <number>");
            }
            continue;
        }

        if input == "/history" {
            if let Some(thread) = session.active_thread() {
                if thread.turns.is_empty() {
                    println!("\n📜 No turns yet in this thread.");
                } else {
                    println!("\n📜 Turn history for thread {} ({} turns):",
                        &thread.id.to_string()[..8], thread.turns.len());
                    for turn in &thread.turns {
                        let state_icon = match turn.state {
                            TurnState::Completed => "✅",
                            TurnState::Processing => "⏳",
                            TurnState::Failed => "❌",
                        };
                        let input_preview = if turn.user_input.len() > 60 {
                            format!("{}...", &turn.user_input[..60])
                        } else {
                            turn.user_input.clone()
                        };
                        println!("\n   Turn #{} {} [{}]",
                            turn.turn_number + 1, state_icon,
                            turn.started_at.format("%H:%M:%S"));
                        println!("     🧑 \"{}\"", input_preview);

                        for tc in &turn.tool_calls {
                            let result_preview = if let Some(ref err) = tc.error {
                                format!("❌ {}", err)
                            } else if let Some(ref res) = tc.result {
                                let s = match res {
                                    serde_json::Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                };
                                if s.len() > 60 { format!("{}...", &s[..60]) } else { s }
                            } else {
                                "⏳ pending".to_string()
                            };
                            println!("     🔧 {}({}) → {}", tc.name, tc.parameters, result_preview);
                        }

                        if let Some(ref response) = turn.response {
                            let resp_preview = if response.len() > 80 {
                                format!("{}...", &response[..80])
                            } else {
                                response.clone()
                            };
                            println!("     🤖 \"{}\"", resp_preview);
                        }
                        if let Some(ref error) = turn.error {
                            println!("     ❌ Error: {}", error);
                        }
                    }
                }
            } else {
                println!("⚠️  No active thread.");
            }
            continue;
        }

        if input == "/session" {
            println!("\n📊 Session info:");
            println!("   ID: {}", session.id);
            println!("   User: {}", session.user_id);
            println!("   Created: {}", session.created_at.format("%Y-%m-%d %H:%M:%S"));
            println!("   Threads: {}", session.threads.len());
            println!("   Total turns: {}", session.threads.values().map(|t| t.turns.len()).sum::<usize>());
            if let Some(thread) = session.active_thread() {
                println!("   Active thread: {} ({} turns, {:?})",
                    &thread.id.to_string()[..8], thread.turns.len(), thread.state);
            }
            println!("   LLM: {}{}", current_model, if use_mock { " (MOCK)" } else { "" });
            continue;
        }

        // ── LLM model commands ──

        if input == "/models" {
            if let Some(ref config) = hai_config {
                println!("\n📋 Available models:");
                println!("   (default) → {}", config.model);
                for (shortcut, model_id) in config.list_models() {
                    let marker = if current_model == model_id { " ← current" } else { "" };
                    println!("   {} → {}{}", shortcut, model_id, marker);
                }
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
                println!("⚠️  No config loaded.");
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

        // ── Process user input through Session/Thread/Turn pipeline ──
        println!("\n🤖 Agent thinking...");
        match process_user_input(
            &mut session,
            llm.as_ref(),
            &registry,
            &system_prompt,
            &loop_config,
            input,
        ).await {
            Ok(text) => {
                println!("\n🤖 Agent: {}", text);
            }
            Err(e) => {
                println!("\n❌ Error: {}", e);
            }
        }
    }
}
