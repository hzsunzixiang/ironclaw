
//! # LLM Types & Provider
//!
//! Corresponds to: `src/llm/provider.rs` in IronClaw.
//! Defines the core types for LLM interaction.

mod openai;
mod provider;

pub use openai::{HaiConfig, OpenAiCompatibleProvider};
pub use provider::LlmProvider;

use serde::{Deserialize, Serialize};

// ============================================================================
// Core LLM Types
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
