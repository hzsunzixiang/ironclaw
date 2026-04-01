
//! # Long-Term Memory Store
//!
//! THIS IS THE KEY ADDITION compared to mini-agent-wasm-session.
//!
//! Maps to IronClaw's `src/workspace/` + `src/tools/builtin/memory.rs`:
//!   - Persistent memory across sessions (survives restarts)
//!   - File-based workspace: MEMORY.md, daily logs, arbitrary paths
//!   - Search across all memory documents
//!   - System prompt injection (MEMORY.md loaded into context)
//!
//! ## IronClaw's Memory Architecture
//!
//! In IronClaw, memory is database-backed (PostgreSQL/LibSQL) with:
//!   - `memory_documents` table: stores files with path, content, metadata
//!   - `memory_chunks` table: chunked content with FTS + vector embeddings
//!   - Hybrid search: Full-Text Search (keyword) + Vector (semantic) via RRF
//!   - Workspace abstraction: read/write/append/search/list operations
//!
//! Here we simplify to filesystem-backed storage with keyword search,
//! preserving the same conceptual model and API surface.
//!
//! ## Key Insight
//!
//! ```text
//! Session memory (Thread/Turn) = SHORT-TERM memory (within a session)
//! Workspace memory (MEMORY.md)  = LONG-TERM memory (across sessions)
//! ```
//!
//! The LLM is stateless. Session gives it short-term memory (conversation
//! history). Workspace gives it long-term memory (facts, preferences,
//! decisions that persist across restarts).
//!
//! ## Workspace Structure
//!
//! ```text
//! workspace/
//! ├── MEMORY.md              ← Curated long-term facts (loaded into system prompt)
//! ├── daily/
//! │   ├── 2026-04-01.md      ← Today's session log
//! │   └── 2026-03-31.md      ← Yesterday's log
//! └── projects/              ← Arbitrary structure
//!     └── alpha/
//!         └── notes.md
//! ```

use chrono::{Local, Utc};
use std::path::{Path, PathBuf};
use tracing::{debug, info, error};

// ============================================================================
// Well-Known Paths (maps to ironclaw's workspace::document::paths)
// ============================================================================

/// Well-known document paths in the workspace.
///
/// Maps to: `src/workspace/document.rs` → `mod paths`
pub mod paths {
    /// Long-term curated memory (loaded into system prompt).
    pub const MEMORY: &str = "MEMORY.md";
    /// Daily logs directory.
    pub const DAILY_DIR: &str = "daily";
}

// ============================================================================
// MemoryDocument — A single document in the workspace
// ============================================================================

/// A document stored in the workspace memory.
///
/// Maps to: `src/workspace/document.rs` → `struct MemoryDocument`
///
/// In IronClaw this is a database row with UUID, user_id, agent_id, etc.
/// Here we simplify to a file on disk.
#[derive(Debug, Clone)]
pub struct MemoryDocument {
    /// File path relative to workspace root (e.g. "MEMORY.md", "daily/2026-04-01.md")
    pub path: String,
    /// Full content of the document.
    pub content: String,
    /// When the document was last modified.
    pub updated_at: chrono::DateTime<Utc>,
}

impl MemoryDocument {
    /// Word count (approximate).
    pub fn word_count(&self) -> usize {
        self.content.split_whitespace().count()
    }
}

// ============================================================================
// SearchResult — A search hit
// ============================================================================

/// A search result from the memory store.
///
/// Maps to: ironclaw's `SearchResult` from hybrid FTS + vector search.
/// Here we use simple keyword matching with a relevance score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Path of the matching document.
    pub path: String,
    /// The matching content snippet.
    pub content: String,
    /// Relevance score (0.0 - 1.0).
    pub score: f64,
}

// ============================================================================
// MemoryStore — The workspace memory manager
// ============================================================================

/// File-system-backed memory store.
///
/// Maps to: `src/workspace/mod.rs` → `struct Workspace`
///
/// In IronClaw, Workspace wraps a database pool + embedding provider.
/// Here we use the local filesystem for simplicity, but the API surface
/// is intentionally similar.
pub struct MemoryStore {
    /// Root directory for workspace files.
    root: PathBuf,
}

impl MemoryStore {
    /// Create a new memory store rooted at the given directory.
    ///
    /// Creates the directory (and MEMORY.md seed) if they don't exist.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        info!(
            root = %root.display(),
            "📂 Initializing memory store"
        );

        // Ensure root directory exists
        if !root.exists() {
            std::fs::create_dir_all(&root).unwrap_or_else(|e| {
                error!(error = %e, "Failed to create workspace directory");
            });
            info!(root = %root.display(), "Created workspace directory");
        }

        // Seed MEMORY.md if it doesn't exist
        let memory_path = root.join(paths::MEMORY);
        if !memory_path.exists() {
            let seed_content = "# Memory\n\n\
                Long-term notes, decisions, and facts worth remembering across sessions.\n\n\
                The agent appends here during conversations. Curate periodically:\n\
                remove stale entries, consolidate duplicates, keep it concise.\n\
                This file is loaded into the system prompt, so brevity matters.\n";
            std::fs::write(&memory_path, seed_content).unwrap_or_else(|e| {
                error!(error = %e, "Failed to seed MEMORY.md");
            });
            info!("📝 Seeded MEMORY.md");
        }

        // Ensure daily/ directory exists
        let daily_dir = root.join(paths::DAILY_DIR);
        if !daily_dir.exists() {
            std::fs::create_dir_all(&daily_dir).unwrap_or_else(|e| {
                error!(error = %e, "Failed to create daily/ directory");
            });
        }

        Self { root }
    }

    // ── Read / Write / Append ──────────────────────────────────────

    /// Read a document by path.
    ///
    /// Maps to: `Workspace::read(path)`
    pub fn read(&self, path: &str) -> Result<MemoryDocument, String> {
        let full_path = self.root.join(path);
        debug!(path = %path, full_path = %full_path.display(), "Reading document");

        if !full_path.exists() {
            return Err(format!("Document not found: {}", path));
        }

        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| format!("Failed to read '{}': {}", path, e))?;

        let metadata = std::fs::metadata(&full_path)
            .map_err(|e| format!("Failed to read metadata for '{}': {}", path, e))?;

        let updated_at = metadata
            .modified()
            .map(|t| chrono::DateTime::<Utc>::from(t))
            .unwrap_or_else(|_| Utc::now());

        Ok(MemoryDocument {
            path: path.to_string(),
            content,
            updated_at,
        })
    }

    /// Write content to a document (creates or replaces).
    ///
    /// Maps to: `Workspace::write(path, content)`
    pub fn write(&self, path: &str, content: &str) -> Result<(), String> {
        let full_path = self.root.join(path);
        info!(path = %path, content_len = content.len(), "📝 Writing document");

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        std::fs::write(&full_path, content)
            .map_err(|e| format!("Failed to write '{}': {}", path, e))?;

        Ok(())
    }

    /// Append content to a document (creates if doesn't exist).
    ///
    /// Maps to: `Workspace::append(path, content)`
    pub fn append(&self, path: &str, content: &str) -> Result<(), String> {
        let full_path = self.root.join(path);
        debug!(path = %path, content_len = content.len(), "📝 Appending to document");

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let existing = if full_path.exists() {
            std::fs::read_to_string(&full_path)
                .map_err(|e| format!("Failed to read '{}': {}", path, e))?
        } else {
            String::new()
        };

        let new_content = if existing.is_empty() {
            content.to_string()
        } else {
            format!("{}\n{}", existing.trim_end(), content)
        };

        std::fs::write(&full_path, new_content)
            .map_err(|e| format!("Failed to write '{}': {}", path, e))?;

        Ok(())
    }

    // ── Convenience Methods ────────────────────────────────────────

    /// Append to MEMORY.md (curated long-term facts).
    ///
    /// Maps to: `Workspace::append_memory(content)`
    pub fn append_memory(&self, content: &str) -> Result<(), String> {
        let entry = format!("\n- {}", content);
        self.append(paths::MEMORY, &entry)
    }

    /// Append to today's daily log.
    ///
    /// Maps to: `Workspace::append_daily_log(content)`
    pub fn append_daily_log(&self, content: &str) -> Result<(), String> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let path = format!("{}/{}.md", paths::DAILY_DIR, today);

        // Create header if file doesn't exist
        let full_path = self.root.join(&path);
        if !full_path.exists() {
            let header = format!("# Daily Log — {}\n", today);
            self.write(&path, &header)?;
        }

        let timestamp = Local::now().format("%H:%M:%S").to_string();
        let entry = format!("\n[{}] {}", timestamp, content);
        self.append(&path, &entry)
    }

    /// Read MEMORY.md content (for system prompt injection).
    ///
    /// Maps to: `Workspace::system_prompt()` (partial — just the memory part)
    pub fn memory_content(&self) -> String {
        match self.read(paths::MEMORY) {
            Ok(doc) => doc.content,
            Err(_) => String::new(),
        }
    }

    // ── Search ─────────────────────────────────────────────────────

    /// Search across all memory documents.
    ///
    /// Maps to: `Workspace::search(query, limit)`
    ///
    /// In IronClaw, this is hybrid FTS + vector search with RRF fusion.
    /// Here we use simple case-insensitive keyword matching with a
    /// relevance score based on keyword density.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        info!(query = %query, limit = limit, "🔍 Searching memory");

        let keywords: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if keywords.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<SearchResult> = Vec::new();

        // Walk all files in workspace
        if let Ok(entries) = self.walk_files() {
            for (path, content) in entries {
                let lower_content = content.to_lowercase();
                let mut match_count = 0;
                for kw in &keywords {
                    match_count += lower_content.matches(kw).count();
                }

                if match_count > 0 {
                    // Score: keyword density (matches per 100 words)
                    let word_count = content.split_whitespace().count().max(1);
                    let score = (match_count as f64 / word_count as f64 * 100.0).min(1.0);

                    // Extract a relevant snippet (first matching line + context)
                    let snippet = self.extract_snippet(&content, &keywords);

                    results.push(SearchResult {
                        path,
                        content: snippet,
                        score,
                    });
                }
            }
        }

        // Sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        info!(
            result_count = results.len(),
            "🔍 Search returned {} result(s)",
            results.len()
        );

        results
    }

    /// List all files in the workspace (path relative to root, content).
    fn walk_files(&self) -> Result<Vec<(String, String)>, String> {
        let mut files = Vec::new();
        self.walk_dir_recursive(&self.root, &mut files)?;
        Ok(files)
    }

    fn walk_dir_recursive(&self, dir: &Path, files: &mut Vec<(String, String)>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("Failed to read directory: {}", e))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.walk_dir_recursive(&path, files)?;
            } else if path.is_file() {
                // Only read text files
                if let Some(ext) = path.extension() {
                    if ext == "md" || ext == "txt" || ext == "json" {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let rel_path = path
                                .strip_prefix(&self.root)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .to_string();
                            files.push((rel_path, content));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Extract a relevant snippet from content around the first keyword match.
    fn extract_snippet(&self, content: &str, keywords: &[String]) -> String {
        let lines: Vec<&str> = content.lines().collect();

        // Find the first line containing a keyword
        for (i, line) in lines.iter().enumerate() {
            let lower_line = line.to_lowercase();
            if keywords.iter().any(|kw| lower_line.contains(kw)) {
                // Return this line + 1 line of context before/after
                let start = i.saturating_sub(1);
                let end = (i + 2).min(lines.len());
                return lines[start..end].join("\n");
            }
        }

        // Fallback: first 3 lines
        lines.iter().take(3).copied().collect::<Vec<_>>().join("\n")
    }

    // ── Tree View ──────────────────────────────────────────────────

    /// List workspace structure as a tree.
    ///
    /// Maps to: `MemoryTreeTool`
    pub fn tree(&self) -> Vec<String> {
        let mut entries = Vec::new();
        self.tree_recursive(&self.root, "", &mut entries);
        entries
    }

    fn tree_recursive(&self, dir: &Path, prefix: &str, entries: &mut Vec<String>) {
        if let Ok(read_dir) = std::fs::read_dir(dir) {
            let mut items: Vec<_> = read_dir.flatten().collect();
            items.sort_by_key(|e| e.file_name());

            for entry in items {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                if path.is_dir() {
                    entries.push(format!("{}{}/", prefix, name));
                    self.tree_recursive(&path, &format!("{}  ", prefix), entries);
                } else {
                    entries.push(format!("{}{}", prefix, name));
                }
            }
        }
    }

    /// Get the workspace root path.
    pub fn root_path(&self) -> &Path {
        &self.root
    }
}

// ============================================================================
// Auto-Memory Extraction
// ============================================================================

/// Analyze a completed turn and extract facts worth remembering.
///
/// This is a heuristic approach — in IronClaw, the LLM itself decides
/// what to remember via memory_write tool calls. Here we provide a
/// simpler automatic extraction as a complement.
///
/// Returns a list of memory entries to save, or empty if nothing notable.
pub fn extract_memories_from_turn(user_input: &str, _response: &str) -> Vec<String> {
    let mut memories = Vec::new();

    // Heuristic 1: User explicitly asks to remember something
    let lower_input = user_input.to_lowercase();
    if lower_input.contains("remember")
        || lower_input.contains("note that")
        || lower_input.contains("keep in mind")
        || lower_input.contains("don't forget")
        || lower_input.contains("save this")
    {
        // Extract the key fact from the user's input
        let fact = user_input
            .replace("remember", "")
            .replace("note that", "")
            .replace("keep in mind", "")
            .replace("don't forget", "")
            .replace("save this", "")
            .replace("please", "")
            .replace("Please", "")
            .trim()
            .to_string();

        if !fact.is_empty() && fact.len() > 5 {
            memories.push(format!("User asked to remember: {}", fact));
        }
    }

    // Heuristic 2: User states a preference
    if lower_input.contains("i prefer")
        || lower_input.contains("i like")
        || lower_input.contains("i always")
        || lower_input.contains("i never")
        || lower_input.contains("my favorite")
        || lower_input.contains("i usually")
    {
        memories.push(format!("User preference: {}", user_input));
    }

    // Heuristic 3: User shares personal info
    if lower_input.contains("my name is")
        || lower_input.contains("i work at")
        || lower_input.contains("i'm a ")
        || lower_input.contains("i am a ")
        || lower_input.contains("my job is")
        || lower_input.contains("i live in")
    {
        memories.push(format!("User info: {}", user_input));
    }

    memories
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_memories_remember() {
        let memories = extract_memories_from_turn(
            "Please remember that the project deadline is March 15th",
            "Got it!",
        );
        assert_eq!(memories.len(), 1);
        assert!(memories[0].contains("deadline"));
    }

    #[test]
    fn test_extract_memories_preference() {
        let memories = extract_memories_from_turn(
            "I prefer dark mode in all my editors",
            "Noted!",
        );
        assert_eq!(memories.len(), 1);
        assert!(memories[0].contains("dark mode"));
    }

    #[test]
    fn test_extract_memories_nothing() {
        let memories = extract_memories_from_turn(
            "What is 42 + 58?",
            "The answer is 100.",
        );
        assert!(memories.is_empty());
    }

    #[test]
    fn test_memory_store_basic() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());

        // Write and read
        store.write("test.md", "Hello, world!").unwrap();
        let doc = store.read("test.md").unwrap();
        assert_eq!(doc.content, "Hello, world!");

        // Append
        store.append("test.md", "More content").unwrap();
        let doc = store.read("test.md").unwrap();
        assert!(doc.content.contains("Hello, world!"));
        assert!(doc.content.contains("More content"));

        // Search
        let results = store.search("hello", 5);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Hello"));
    }

    #[test]
    fn test_memory_store_append_memory() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());

        store.append_memory("User prefers dark mode").unwrap();
        let content = store.memory_content();
        assert!(content.contains("User prefers dark mode"));
    }
}
