
//! # LLM Provider Trait
//!
//! Corresponds to: `src/llm/provider.rs` → `trait LlmProvider` in IronClaw.
//!
//! In IronClaw, this has `complete()` and `complete_with_tools()` methods,
//! plus cost tracking, model switching, etc. We merge into one method.

use async_trait::async_trait;

use super::{ChatMessage, LlmResponse, ToolDefinition};

/// Trait for LLM providers.
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
