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

use tracing::{debug, info, trace, warn, error};

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
        debug!("Creating new ToolRegistry");
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        info!(tool_name = %tool.name(), "Registering tool: '{}'", tool.name());
        debug!(
            tool_name = %tool.name(),
            description = %tool.description(),
            schema = %serde_json::to_string_pretty(&tool.parameters_schema()).unwrap_or_default(),
            "Tool details"
        );
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
    info!(tool = %tool_name, "🔍 Looking up tool in registry");

    let tool = registry
        .get(tool_name)
        .ok_or_else(|| {
            error!(tool = %tool_name, "Tool '{}' not found in registry", tool_name);
            format!("Tool '{}' not found", tool_name)
        })?;

    debug!(
        tool = %tool_name,
        params = %serde_json::to_string_pretty(&params).unwrap_or_default(),
        "Tool found, preparing execution"
    );
    println!(
        "  ⚙️  Executing tool: {} with params: {}",
        tool_name, params
    );

    let timeout = Duration::from_secs(30);
    info!(
        tool = %tool_name,
        timeout_secs = 30,
        "⏱️  Executing tool with {}s timeout",
        30
    );

    let exec_start = std::time::Instant::now();
    let result = tokio::time::timeout(timeout, tool.execute(params)).await;
    let exec_elapsed = exec_start.elapsed();

    match result {
        Ok(Ok(output)) => {
            info!(
                tool = %tool_name,
                elapsed_ms = exec_elapsed.as_millis(),
                "✅ Tool execution succeeded in {}ms",
                exec_elapsed.as_millis()
            );
            trace!(
                tool = %tool_name,
                raw_output = %serde_json::to_string_pretty(&output.result).unwrap_or_default(),
                "Raw tool output"
            );
            let serialized = serde_json::to_string_pretty(&output.result)
                .map_err(|e| {
                    error!(error = %e, "Failed to serialize tool result");
                    format!("Failed to serialize result: {}", e)
                })?;
            debug!(
                tool = %tool_name,
                serialized_len = serialized.len(),
                serialized = %serialized,
                "Serialized tool output ({} chars)",
                serialized.len()
            );
            Ok(serialized)
        }
        Ok(Err(e)) => {
            error!(
                tool = %tool_name,
                elapsed_ms = exec_elapsed.as_millis(),
                error = %e,
                "❌ Tool execution failed: {}",
                e
            );
            Err(format!("Tool execution failed: {}", e))
        }
        Err(_) => {
            error!(
                tool = %tool_name,
                timeout_secs = 30,
                "⏰ Tool timed out after {}s",
                30
            );
            Err(format!(
                "Tool '{}' timed out after {:?}",
                tool_name, timeout
            ))
        }
    }
}

/// Process a tool result into a ChatMessage.
pub fn process_tool_result(
    tool_name: &str,
    tool_call_id: &str,
    result: &Result<String, String>,
) -> ChatMessage {
    let content = match result {
        Ok(output) => {
            let wrapped = format!("<tool_output>{}</tool_output>", output);
            debug!(
                tool = %tool_name,
                tool_call_id = %tool_call_id,
                content_len = wrapped.len(),
                "Wrapping tool output in <tool_output> tags ({} chars)",
                wrapped.len()
            );
            trace!(
                tool = %tool_name,
                wrapped_content = %wrapped,
                "Full wrapped tool result"
            );
            wrapped
        }
        Err(e) => {
            let error_content = format!("Error: {}", e);
            warn!(
                tool = %tool_name,
                tool_call_id = %tool_call_id,
                error = %e,
                "Creating error tool result message"
            );
            error_content
        }
    };
    info!(
        tool = %tool_name,
        tool_call_id = %tool_call_id,
        is_error = result.is_err(),
        "📝 Creating tool result ChatMessage (role=Tool)"
    );
    ChatMessage::tool_result(tool_call_id, tool_name, content)
}
