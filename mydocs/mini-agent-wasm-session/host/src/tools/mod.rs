//! # Tool Trait, Registry & Execution Pipeline
//!
//! Corresponds to: `src/tools/tool.rs` + `src/tools/execute.rs` in IronClaw.
//!
//! Defines the abstract Tool trait, the ToolRegistry for managing tools,
//! and the execution pipeline with timeout safety.
//!
//! The WASM sandbox implementation lives in `wasm.rs`.

mod wasm;

pub use wasm::{WasmTool, WasmToolEngine};

use async_trait::async_trait;
use serde::Serialize;
use std::time::Duration;

use crate::llm::{ChatMessage, ToolDefinition};

// ============================================================================
// Tool Trait & Registry
// ============================================================================

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize)]
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
#[derive(Default)]
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
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect()
    }
}

// ════════════════════════════════════════════════════════════════════
// Tool Execution Pipeline
// ════════════════════════════════════════════════════════════════════

/// Execute a tool with timeout.
/// Maps to: `src/tools/execute.rs` → `execute_tool_with_safety()`
pub async fn execute_tool_with_safety(
    registry: &ToolRegistry,
    tool_name: &str,
    params: serde_json::Value,
) -> Result<String, String> {
    let tool = registry
        .get(tool_name)
        .ok_or_else(|| format!("Tool '{}' not found", tool_name))?;

    println!(
        "  ⚙️  Executing tool: {} with params: {}",
        tool_name, params
    );

    let timeout = Duration::from_secs(30);
    let result = tokio::time::timeout(timeout, tool.execute(params)).await;

    match result {
        Ok(Ok(output)) => serde_json::to_string_pretty(&output.result)
            .map_err(|e| format!("Failed to serialize result: {}", e)),
        Ok(Err(e)) => Err(format!("Tool execution failed: {}", e)),
        Err(_) => Err(format!(
            "Tool '{}' timed out after {:?}",
            tool_name, timeout
        )),
    }
}

/// Process a tool result into a ChatMessage.
pub fn process_tool_result(
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
