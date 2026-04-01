
//! # Native Memory Tools
//!
//! Maps to: `src/tools/builtin/memory.rs` in IronClaw.
//!
//! These are NATIVE tools (not WASM sandboxed) that provide the agent
//! with persistent memory capabilities:
//!
//! | Tool            | IronClaw equivalent | Purpose                              |
//! |-----------------|---------------------|--------------------------------------|
//! | memory_search   | MemorySearchTool    | Search past memories and context     |
//! | memory_write    | MemoryWriteTool     | Write facts to persistent memory     |
//! | memory_read     | MemoryReadTool      | Read a specific memory document      |
//!
//! ## Key Difference: Native vs WASM Tools
//!
//! ```text
//! WASM tools (calculator):
//!   - Run in sandbox, fully isolated
//!   - Cannot access filesystem, network, etc.
//!   - Safe to run untrusted code
//!
//! Native tools (memory_search, memory_write):
//!   - Run in host process, full access
//!   - Can read/write the workspace filesystem
//!   - Trusted code only (part of the agent)
//! ```
//!
//! In IronClaw, memory tools access a database via the Workspace abstraction.
//! Here we access the filesystem via MemoryStore.

use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::info;

use super::{Tool, ToolOutput};
use crate::memory::MemoryStore;

// ============================================================================
// MemorySearchTool — Search past memories
// ============================================================================

/// Tool for searching workspace memory.
///
/// Maps to: `src/tools/builtin/memory.rs` → `MemorySearchTool`
///
/// Performs keyword search across all memory documents.
/// The agent should call this before answering questions about
/// prior work, decisions, preferences, or historical context.
pub struct MemorySearchTool {
    store: Arc<RwLock<MemoryStore>>,
}

impl MemorySearchTool {
    pub fn new(store: Arc<RwLock<MemoryStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search past memories, decisions, and context. MUST be called before answering \
         questions about prior work, decisions, dates, people, preferences, or todos. \
         Returns relevant snippets with relevance scores."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Use natural language to describe what you're looking for."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5, max: 20)",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter: query".to_string())?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(20) as usize;

        info!(query = %query, limit = limit, "🔍 memory_search tool called");

        let store = self.store.read().await;
        let results = store.search(query, limit);

        let output = serde_json::json!({
            "query": query,
            "results": results.iter().map(|r| serde_json::json!({
                "content": r.content,
                "score": r.score,
                "path": r.path,
            })).collect::<Vec<_>>(),
            "result_count": results.len(),
        });

        Ok(ToolOutput { result: output })
    }
}

// ============================================================================
// MemoryWriteTool — Write to persistent memory
// ============================================================================

/// Tool for writing to workspace memory.
///
/// Maps to: `src/tools/builtin/memory.rs` → `MemoryWriteTool`
///
/// Use this to persist important information that should be remembered
/// across sessions: decisions, preferences, facts, lessons learned.
pub struct MemoryWriteTool {
    store: Arc<RwLock<MemoryStore>>,
}

impl MemoryWriteTool {
    pub fn new(store: Arc<RwLock<MemoryStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Write to persistent memory. Use for important facts, decisions, preferences, \
         or lessons learned that should be remembered across sessions. \
         Targets: 'memory' for curated long-term facts (MEMORY.md), \
         'daily_log' for timestamped session notes, \
         or provide a custom workspace path like 'projects/alpha/notes.md'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to write to memory. Be concise but include relevant context."
                },
                "target": {
                    "type": "string",
                    "description": "Where to write: 'memory' for MEMORY.md, 'daily_log' for today's log, or a path like 'projects/alpha/notes.md'",
                    "default": "daily_log"
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to existing content. If false, replace entirely.",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter: content".to_string())?;

        if content.trim().is_empty() {
            return Err("content cannot be empty".to_string());
        }

        let target = params
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("daily_log");

        let append = params
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        info!(
            target = %target,
            content_len = content.len(),
            append = append,
            "📝 memory_write tool called"
        );

        let store = self.store.write().await;

        match target {
            "memory" => {
                if append {
                    store.append_memory(content)?;
                } else {
                    store.write(crate::memory::paths::MEMORY, content)?;
                }
            }
            "daily_log" => {
                store.append_daily_log(content)?;
            }
            path => {
                if append {
                    store.append(path, content)?;
                } else {
                    store.write(path, content)?;
                }
            }
        }

        let resolved_path = match target {
            "memory" => crate::memory::paths::MEMORY.to_string(),
            "daily_log" => format!(
                "daily/{}.md",
                chrono::Local::now().format("%Y-%m-%d")
            ),
            path => path.to_string(),
        };

        let output = serde_json::json!({
            "status": "written",
            "path": resolved_path,
            "append": append,
            "content_length": content.len(),
        });

        Ok(ToolOutput { result: output })
    }
}

// ============================================================================
// MemoryReadTool — Read a specific memory document
// ============================================================================

/// Tool for reading workspace files.
///
/// Maps to: `src/tools/builtin/memory.rs` → `MemoryReadTool`
pub struct MemoryReadTool {
    store: Arc<RwLock<MemoryStore>>,
}

impl MemoryReadTool {
    pub fn new(store: Arc<RwLock<MemoryStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &str {
        "memory_read"
    }

    fn description(&self) -> &str {
        "Read a file from the workspace memory. Use this to read files found by \
         memory_search. Works with MEMORY.md, daily logs, or any custom workspace path."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (e.g., 'MEMORY.md', 'daily/2026-04-01.md', 'projects/alpha/notes.md')"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter: path".to_string())?;

        info!(path = %path, "📖 memory_read tool called");

        let store = self.store.read().await;
        let doc = store.read(path)?;

        let output = serde_json::json!({
            "path": doc.path,
            "content": doc.content,
            "word_count": doc.word_count(),
            "updated_at": doc.updated_at.to_rfc3339(),
        });

        Ok(ToolOutput { result: output })
    }
}
