//! # Real OpenAI-Compatible LLM Provider
//!
//! Reads configuration from ./HAI_WOA.json (or ~/HAI_WOA.json as fallback) and calls a real LLM API.
//! Supports any OpenAI-compatible endpoint (HaiHub, OpenRouter, etc.)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use super::provider::LlmProvider;
use super::{ChatMessage, FinishReason, LlmOutput, LlmResponse, Role, ToolCall, ToolDefinition};

// ============================================================================
// HaiConfig — Configuration from HAI_WOA.json
// ============================================================================

/// Configuration loaded from ./HAI_WOA.json (fallback: ~/HAI_WOA.json)
#[derive(Debug, Deserialize)]
pub struct HaiConfig {
    /// Default model ID (e.g. "openai/DeepSeek-V3-0324")
    pub model: String,
    /// Named model shortcuts
    #[serde(default)]
    pub models: HashMap<String, ModelEntry>,
    /// Environment variables (OPENAI_API_KEY, OPENAI_BASE_URL)
    pub env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct ModelEntry {
    pub id: String,
}

impl HaiConfig {
    /// Load from ./HAI_WOA.json (current directory first, fallback to ~/HAI_WOA.json)
    pub fn load() -> Result<Self, String> {
        // Try current directory first, then fall back to home directory
        let local_path = std::path::PathBuf::from("HAI_WOA.json");
        let path = if local_path.exists() {
            local_path
        } else {
            let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
            home.join("HAI_WOA.json")
        };
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
    }

    pub fn api_key(&self) -> Result<String, String> {
        self.env
            .get("OPENAI_API_KEY")
            .cloned()
            .ok_or_else(|| "OPENAI_API_KEY not found in HAI_WOA.json env".to_string())
    }

    pub fn base_url(&self) -> Result<String, String> {
        self.env
            .get("OPENAI_BASE_URL")
            .cloned()
            .ok_or_else(|| "OPENAI_BASE_URL not found in HAI_WOA.json env".to_string())
    }

    /// Resolve a model name: check shortcuts first, then use as-is.
    /// Strips "openai/" prefix since HaiHub API doesn't need it.
    pub fn resolve_model(&self, name: Option<&str>) -> String {
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

    /// List available model shortcuts
    pub fn list_models(&self) -> Vec<(&str, &str)> {
        let mut models: Vec<_> = self
            .models
            .iter()
            .map(|(k, v)| (k.as_str(), v.id.as_str()))
            .collect();
        models.sort_by_key(|(k, _)| *k);
        models
    }
}

// ============================================================================
// OpenAI-Compatible Provider
// ============================================================================

/// Real OpenAI-compatible LLM provider.
/// Calls /v1/chat/completions with tool support.
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiCompatibleProvider {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self {
            client,
            api_key,
            base_url,
            model,
        }
    }
}

// ============================================================================
// OpenAI API Types (Request)
// ============================================================================

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

// ============================================================================
// OpenAI API Types (Response)
// ============================================================================

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

// ============================================================================
// Conversion Helpers
// ============================================================================

impl OpenAiCompatibleProvider {
    /// Convert our ChatMessage list to OpenAI format
    fn convert_messages(messages: &[ChatMessage]) -> Vec<OpenAiMessage> {
        messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                let tool_calls_out = m.tool_calls.as_ref().map(|tcs| {
                    tcs.iter()
                        .map(|tc| OpenAiToolCallOut {
                            id: tc.id.clone(),
                            call_type: "function".to_string(),
                            function: OpenAiFunctionCallOut {
                                name: tc.name.clone(),
                                arguments: serde_json::to_string(&tc.arguments)
                                    .unwrap_or_default(),
                            },
                        })
                        .collect()
                });
                OpenAiMessage {
                    role: role.to_string(),
                    content: m.content.clone(),
                    tool_call_id: m.tool_call_id.clone(),
                    name: m.name.clone(),
                    tool_calls: tool_calls_out,
                }
            })
            .collect()
    }

    /// Convert our ToolDefinition list to OpenAI format
    fn convert_tools(tools: &[ToolDefinition]) -> Vec<OpenAiTool> {
        tools
            .iter()
            .map(|t| OpenAiTool {
                tool_type: "function".to_string(),
                function: OpenAiFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect()
    }
}

// ============================================================================
// LlmProvider Implementation
// ============================================================================

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
            tool_choice: if tools.is_empty() {
                None
            } else {
                Some("auto".to_string())
            },
        };

        let resp = self
            .client
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

        let openai_resp: OpenAiResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        let choice = openai_resp
            .choices
            .into_iter()
            .next()
            .ok_or("No choices in response")?;

        let finish = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("length") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        // Check if there are tool calls
        if let Some(tool_calls_in) = choice.message.tool_calls {
            if !tool_calls_in.is_empty() {
                let tool_calls: Vec<ToolCall> = tool_calls_in
                    .into_iter()
                    .map(|tc| {
                        let arguments: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                        ToolCall {
                            id: tc.id,
                            name: tc.function.name,
                            arguments,
                        }
                    })
                    .collect();

                return Ok(LlmResponse {
                    result: LlmOutput::ToolCalls {
                        tool_calls,
                        content: choice.message.content,
                    },
                    finish_reason: FinishReason::ToolUse,
                });
            }
        }

        // Text response
        let text = choice.message.content.unwrap_or_default();
        Ok(LlmResponse {
            result: LlmOutput::Text(text),
            finish_reason: finish,
        })
    }
}
