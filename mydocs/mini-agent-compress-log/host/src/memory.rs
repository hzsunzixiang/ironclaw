
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

    /// Append to MEMORY.md (curated long-term facts) with deduplication.
    ///
    /// Maps to: `Workspace::append_memory(content)`
    ///
    /// ## What's new (compared to mini-agent-memory):
    ///
    /// Checks existing MEMORY.md content for similar entries before appending.
    /// This prevents the "favorite color is blue" × 25 problem observed in
    /// the original trace logs.
    ///
    /// In IronClaw, deduplication is handled at the database level with
    /// chunk-based FTS + vector similarity. Here we use simple keyword
    /// overlap scoring as a lightweight approximation.
    pub fn append_memory(&self, content: &str) -> Result<(), String> {
        // Read existing memory content
        let existing = self.memory_content();

        // Check for duplicates against existing entries
        if self.is_duplicate_memory(content, &existing) {
            info!(
                content = %crate::utils::truncate_str(content, 60),
                "🧠 Skipping duplicate memory (already exists)"
            );
            return Ok(());
        }

        let entry = format!("\n- {}", content);
        self.append(paths::MEMORY, &entry)
    }

    /// Check if a new memory entry is a duplicate of existing content.
    ///
    /// Uses keyword overlap scoring: if the new entry shares >70% of its
    /// significant words with any existing entry, it's considered a duplicate.
    ///
    /// Maps to: IronClaw's chunk deduplication by path + content similarity.
    fn is_duplicate_memory(&self, new_entry: &str, existing_content: &str) -> bool {
        let new_words = Self::significant_words(new_entry);
        if new_words.is_empty() {
            return false;
        }

        // Check against each existing line that starts with "- "
        for line in existing_content.lines() {
            let line = line.trim();
            if !line.starts_with("- ") && !line.starts_with("* ") {
                continue;
            }
            let existing_entry = &line[2..]; // strip "- " or "* "
            let score = Self::similarity_score(&new_words, existing_entry);
            if score > 0.70 {
                debug!(
                    new = %crate::utils::truncate_str(new_entry, 40),
                    existing = %crate::utils::truncate_str(existing_entry, 40),
                    score = score,
                    "Duplicate detected (similarity={:.2})",
                    score
                );
                return true;
            }
        }
        false
    }

    /// Extract significant words from text (lowercase, no stop words, len > 2).
    fn significant_words(text: &str) -> Vec<String> {
        const STOP_WORDS: &[&str] = &[
            "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
            "have", "has", "had", "do", "does", "did", "will", "would", "could",
            "should", "may", "might", "shall", "can", "need", "dare", "ought",
            "used", "to", "of", "in", "for", "on", "with", "at", "by", "from",
            "as", "into", "through", "during", "before", "after", "above", "below",
            "between", "out", "off", "over", "under", "again", "further", "then",
            "once", "here", "there", "when", "where", "why", "how", "all", "each",
            "every", "both", "few", "more", "most", "other", "some", "such", "no",
            "nor", "not", "only", "own", "same", "so", "than", "too", "very",
            "just", "because", "but", "and", "or", "if", "while", "that", "this",
            "these", "those", "it", "its", "my", "your", "his", "her", "our",
            "their", "what", "which", "who", "whom", "i", "me", "we", "us",
            "you", "he", "she", "they", "them", "user", "asked", "remember",
            "preference", "info", "note",
        ];

        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !STOP_WORDS.contains(w))
            .map(|w| w.to_string())
            .collect()
    }

    /// Calculate similarity score between a set of words and a text string.
    /// Returns 0.0 - 1.0 based on what fraction of `new_words` appear in `existing_text`.
    fn similarity_score(new_words: &[String], existing_text: &str) -> f64 {
        if new_words.is_empty() {
            return 0.0;
        }
        let existing_lower = existing_text.to_lowercase();
        let matches = new_words
            .iter()
            .filter(|w| existing_lower.contains(w.as_str()))
            .count();
        matches as f64 / new_words.len() as f64
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

    /// Read MEMORY.md content, truncated to a maximum word count.
    ///
    /// Maps to: IronClaw's context window management — prevents MEMORY.md
    /// from consuming too much of the prompt token budget.
    ///
    /// ## What's new (compared to mini-agent-memory):
    ///
    /// The original had no limit on MEMORY.md size injected into the system
    /// prompt. As memories accumulated, prompt tokens grew unboundedly.
    /// This method caps the injected content to `max_words` words, keeping
    /// the most recent entries (bottom of the file) which are typically
    /// most relevant.
    pub fn memory_content_truncated(&self, max_words: usize) -> String {
        let content = self.memory_content();
        let word_count = content.split_whitespace().count();

        if word_count <= max_words {
            return content;
        }

        info!(
            word_count = word_count,
            max_words = max_words,
            "📏 Truncating MEMORY.md for prompt injection ({} → {} words)",
            word_count, max_words
        );

        // Strategy: keep the header + most recent entries (bottom of file).
        // Split into header (first few lines) and entries.
        let lines: Vec<&str> = content.lines().collect();

        // Find where entries start (after header comments)
        let header_end = lines
            .iter()
            .position(|l| l.starts_with("- ") || l.starts_with("* "))
            .unwrap_or(0);

        let header: String = lines[..header_end].join("\n");
        let entries: Vec<&str> = lines[header_end..].to_vec();

        // Keep entries from the end (most recent) until we hit the word limit
        let header_words = header.split_whitespace().count();
        let budget = max_words.saturating_sub(header_words + 10); // 10 words for truncation notice

        let mut kept_entries: Vec<&str> = Vec::new();
        let mut used_words = 0;
        for entry in entries.iter().rev() {
            let entry_words = entry.split_whitespace().count();
            if used_words + entry_words > budget {
                break;
            }
            kept_entries.push(entry);
            used_words += entry_words;
        }
        kept_entries.reverse();

        let truncated_count = entries.len() - kept_entries.len();
        if truncated_count > 0 {
            format!(
                "{}\n\n[... {} older entries truncated for brevity ...]\n\n{}",
                header,
                truncated_count,
                kept_entries.join("\n")
            )
        } else {
            content
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
// Auto-Memory Extraction (IMPROVED)
// ============================================================================

/// Analyze a completed turn and extract facts worth remembering.
///
/// ## What's new (compared to mini-agent-memory):
///
/// 1. **Question filtering**: Sentences ending with `?` are no longer
///    extracted as preferences (fixes "User preference: What programming
///    languages do I prefer?" bug).
///
/// 2. **Response analysis**: Now examines the LLM response (previously
///    the `_response` parameter was unused) to extract confirmed facts.
///
/// 3. **Better fact extraction**: Strips more noise words and produces
///    cleaner memory entries.
///
/// Maps to: IronClaw's passive profile building (AGENTS.md guidelines).
/// In IronClaw, the LLM itself decides what to remember via memory_write
/// tool calls. This heuristic extraction is a simpler complement.
pub fn extract_memories_from_turn(user_input: &str, response: &str) -> Vec<String> {
    let mut memories = Vec::new();
    let lower_input = user_input.to_lowercase();

    // IMPROVEMENT: Skip extraction if the input is a question.
    // This prevents storing "User preference: What programming languages do I prefer?"
    let is_question = user_input.trim().ends_with('?')
        || lower_input.starts_with("what ")
        || lower_input.starts_with("who ")
        || lower_input.starts_with("where ")
        || lower_input.starts_with("when ")
        || lower_input.starts_with("how ")
        || lower_input.starts_with("why ")
        || lower_input.starts_with("do you ")
        || lower_input.starts_with("can you ")
        || lower_input.starts_with("could you ");

    if is_question {
        debug!(
            input = %crate::utils::truncate_str(user_input, 60),
            "Skipping memory extraction for question input"
        );
        return memories;
    }

    // Heuristic 1: User explicitly asks to remember something
    if lower_input.contains("remember")
        || lower_input.contains("note that")
        || lower_input.contains("keep in mind")
        || lower_input.contains("don't forget")
        || lower_input.contains("save this")
    {
        // Extract the key fact from the user's input
        let fact = user_input
            .replace("remember", "")
            .replace("Remember", "")
            .replace("note that", "")
            .replace("Note that", "")
            .replace("keep in mind", "")
            .replace("don't forget", "")
            .replace("save this", "")
            .replace("please", "")
            .replace("Please", "")
            .replace("that", "")
            .replace("That", "")
            .trim()
            .to_string();

        if !fact.is_empty() && fact.len() > 5 {
            memories.push(format!("User asked to remember: {}", fact.trim()));
        }
    }

    // Heuristic 2: User states a preference (NOT a question)
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

    // IMPROVEMENT: Heuristic 4 — Extract confirmed facts from LLM response.
    // If the LLM response confirms storing something (and we didn't already
    // extract from user input), try to capture it.
    if memories.is_empty() && !response.is_empty() {
        let lower_response = response.to_lowercase();
        if lower_response.contains("i've noted")
            || lower_response.contains("i've saved")
            || lower_response.contains("i'll remember")
            || lower_response.contains("stored in my")
            || lower_response.contains("saved to memory")
        {
            // The LLM confirmed saving something — the user input is the fact
            if user_input.len() > 10 {
                memories.push(format!("Noted from conversation: {}", user_input));
            }
        }
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

    // NEW: Test that questions are filtered out
    #[test]
    fn test_extract_memories_question_filtered() {
        // This was the bug: "What programming languages do I prefer?" was stored
        let memories = extract_memories_from_turn(
            "What programming languages do I prefer?",
            "You prefer Rust for backend and TypeScript for frontend.",
        );
        assert!(memories.is_empty(), "Questions should not be extracted as memories");
    }

    #[test]
    fn test_extract_memories_question_with_keyword() {
        // "Do you remember my name?" should NOT be extracted
        let memories = extract_memories_from_turn(
            "Do you remember my name?",
            "Yes, your name is Erick.",
        );
        assert!(memories.is_empty(), "Questions with 'remember' should not be extracted");
    }

    // NEW: Test deduplication
    #[test]
    fn test_duplicate_detection() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());

        // First write should succeed
        store.append_memory("User prefers Rust for backend development").unwrap();
        let content = store.memory_content();
        assert!(content.contains("Rust for backend"));

        // Duplicate write should be skipped
        store.append_memory("User prefers Rust for backend development").unwrap();
        let content = store.memory_content();
        // Count occurrences — should be exactly 1
        let count = content.matches("Rust for backend").count();
        assert_eq!(count, 1, "Duplicate should have been skipped");
    }

    #[test]
    fn test_similar_but_not_duplicate() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());

        store.append_memory("User prefers Rust for backend development").unwrap();
        // Different enough to not be a duplicate
        store.append_memory("Project Phoenix uses PostgreSQL database").unwrap();
        let content = store.memory_content();
        assert!(content.contains("Rust for backend"));
        assert!(content.contains("PostgreSQL"));
    }

    // NEW: Test memory truncation
    #[test]
    fn test_memory_content_truncated() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());

        // Add many entries
        for i in 0..50 {
            // Use unique entries to bypass dedup
            store.append(&format!("MEMORY.md"), &format!("\n- Unique fact number {} about topic {}", i, i * 7)).unwrap();
        }

        let full = store.memory_content();
        let truncated = store.memory_content_truncated(50);

        // Truncated should be shorter
        let full_words = full.split_whitespace().count();
        let trunc_words = truncated.split_whitespace().count();
        assert!(full_words > 50, "Full content should exceed 50 words");
        assert!(trunc_words <= 60, "Truncated content should be around 50 words (got {})", trunc_words);

        // Should contain truncation notice
        assert!(truncated.contains("older entries truncated"));
    }

    #[test]
    fn test_similarity_score() {
        let words = MemoryStore::significant_words("User prefers Rust for backend development");
        assert!(words.contains(&"rust".to_string()));
        assert!(words.contains(&"backend".to_string()));
        assert!(words.contains(&"development".to_string()));
        // "user" and "for" should be filtered as stop words
        assert!(!words.contains(&"for".to_string()));

        let score = MemoryStore::similarity_score(&words, "Rust for backend development is preferred");
        assert!(score > 0.7, "Similar text should have high score, got {}", score);

        let score2 = MemoryStore::similarity_score(&words, "PostgreSQL database on CentOS");
        assert!(score2 < 0.3, "Different text should have low score, got {}", score2);
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
