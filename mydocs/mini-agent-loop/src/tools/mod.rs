
//! # Tool Trait, Registry & Execution Pipeline
//!
//! Corresponds to: `src/tools/tool.rs` + `src/tools/execute.rs` + `src/tools/registry.rs`
//!
//! In IronClaw, Tool is a rich trait with approval, rate limiting, risk levels,
//! WASM support, etc. Here we keep only: name, description, schema, execute.

mod calculator;

pub use calculator::CalculatorTool;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::llm::{ChatMessage, ToolDefinition};

// ============================================================================
// Tool Trait
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

// ============================================================================
// Tool Registry
// ============================================================================

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

// ============================================================================
// Tool Execution Pipeline
// ============================================================================

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
pub async fn execute_tool_with_safety(
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
