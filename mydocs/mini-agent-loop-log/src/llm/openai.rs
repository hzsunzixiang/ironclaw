//! # Real OpenAI-Compatible LLM Provider
//!
//! Reads configuration from ./HAI_WOA.json and calls a real LLM API.
//! Supports any OpenAI-compatible endpoint (HaiHub, OpenRouter, etc.)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info, trace, warn, error};

use super::provider::LlmProvider;
use super::{ChatMessage, FinishReason, LlmOutput, LlmResponse, Role, ToolCall, ToolDefinition};

// ============================================================================
// HaiConfig — Configuration from ./HAI_WOA.json
// ============================================================================

/// Configuration loaded from ./HAI_WOA.json
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
    /// Load from ./HAI_WOA.json (project directory)
    pub fn load() -> Result<Self, String> {
        let path = std::env::current_dir()
            .map_err(|e| format!("Cannot determine current directory: {}", e))?
            .join("HAI_WOA.json");
        debug!(path = %path.display(), "Loading config file");
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        trace!(content_len = content.len(), "Config file read successfully");
        let config: Self = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
        debug!(
            model = %config.model,
            model_count = config.models.len(),
            "Config parsed: default model='{}', {} model shortcuts",
            config.model, config.models.len()
        );
        Ok(config)
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
        // HaiHub API uses model names without "openai/" prefix
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
        info!(
            model = %model,
            base_url = %base_url,
            api_key_prefix = %format!("{}...", &api_key[..std::cmp::min(8, api_key.len())]),
            "Creating OpenAI-compatible provider (timeout=120s)"
        );
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
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
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
                                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                            },
                        })
                        .collect()
                });
                OpenAiMessage {
                    role: role.to_string(),
                    content: if m.content.is_empty() {
                        None
                    } else {
                        Some(m.content.clone())
                    },
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
        info!(
            url = %url,
            model = %self.model,
            message_count = messages.len(),
            tool_count = tools.len(),
            "📤 Preparing LLM API request"
        );

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

        // Log the full request body at debug level
        debug!(
            request_body = %serde_json::to_string_pretty(&body).unwrap_or_else(|_| "<serialization error>".to_string()),
            "📤 Full OpenAI API request body"
        );
        // Log a summary at debug level
        debug!(
            model = %body.model,
            message_count = body.messages.len(),
            tool_count = body.tools.len(),
            tool_choice = ?body.tool_choice,
            messages_roles = %body.messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>().join(" → "),
            "Request summary: {} messages [{}], {} tools",
            body.messages.len(),
            body.messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>().join(" → "),
            body.tools.len()
        );

        info!("🌐 Sending HTTP POST to {}...", url);
        let http_start = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "HTTP request failed");
                format!("HTTP request failed: {}", e)
            })?;
        let http_elapsed = http_start.elapsed();

        let status = resp.status();
        info!(
            status = %status,
            elapsed_ms = http_elapsed.as_millis(),
            "📥 HTTP response received: {} in {}ms",
            status, http_elapsed.as_millis()
        );

        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            error!(
                status = %status,
                error_body = %error_body,
                "API error response"
            );
            return Err(format!("API returned {}: {}", status, error_body));
        }

        // Read raw response for logging
        let response_text = resp.text().await
            .map_err(|e| format!("Failed to read response body: {}", e))?;
        debug!(
            response_body = %response_text,
            response_len = response_text.len(),
            "📥 Raw API response body ({} bytes)", response_text.len()
        );

        let openai_resp: OpenAiResponse = serde_json::from_str(&response_text)
            .map_err(|e| {
                error!(error = %e, response = %response_text, "Failed to parse response JSON");
                format!("Failed to parse response: {}", e)
            })?;

        debug!(
            choice_count = openai_resp.choices.len(),
            "Parsed response: {} choice(s)",
            openai_resp.choices.len()
        );

        let choice = openai_resp
            .choices
            .into_iter()
            .next()
            .ok_or("No choices in response")?;

        let finish_reason_raw = choice.finish_reason.as_deref().unwrap_or("unknown");
        let finish = match finish_reason_raw {
            "stop" => FinishReason::Stop,
            "tool_calls" => FinishReason::ToolUse,
            "length" => FinishReason::Length,
            other => {
                warn!(finish_reason = %other, "Unknown finish_reason '{}', defaulting to Stop", other);
                FinishReason::Stop
            }
        };
        debug!(
            finish_reason = %finish_reason_raw,
            has_content = choice.message.content.is_some(),
            has_tool_calls = choice.message.tool_calls.is_some(),
            "Response choice: finish_reason={}, has_content={}, has_tool_calls={}",
            finish_reason_raw,
            choice.message.content.is_some(),
            choice.message.tool_calls.is_some()
        );

        // Check if there are tool calls
        if let Some(tool_calls_in) = choice.message.tool_calls {
            if !tool_calls_in.is_empty() {
                info!(
                    tool_call_count = tool_calls_in.len(),
                    "🔧 LLM requested {} tool call(s)",
                    tool_calls_in.len()
                );
                let tool_calls: Vec<ToolCall> = tool_calls_in
                    .into_iter()
                    .enumerate()
                    .map(|(i, tc)| {
                        debug!(
                            index = i,
                            id = %tc.id,
                            function_name = %tc.function.name,
                            raw_arguments = %tc.function.arguments,
                            "Parsing tool call [{}]: {} (id={})",
                            i, tc.function.name, tc.id
                        );
                        let arguments: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or_else(|e| {
                                    warn!(
                                        error = %e,
                                        raw = %tc.function.arguments,
                                        "Failed to parse tool arguments, using empty object"
                                    );
                                    serde_json::Value::Object(serde_json::Map::new())
                                });
                        trace!(
                            parsed_arguments = %serde_json::to_string_pretty(&arguments).unwrap_or_default(),
                            "Parsed tool arguments for '{}'",
                            tc.function.name
                        );
                        ToolCall {
                            id: tc.id,
                            name: tc.function.name,
                            arguments,
                        }
                    })
                    .collect();

                if let Some(ref content) = choice.message.content {
                    debug!(content = %content, "Assistant also returned text content alongside tool calls");
                }

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
        info!(
            text_len = text.len(),
            "💬 LLM returned text response ({} chars)",
            text.len()
        );
        debug!(text = %text, "Full LLM text response");
        Ok(LlmResponse {
            result: LlmOutput::Text(text),
            finish_reason: finish,
        })
    }
}
