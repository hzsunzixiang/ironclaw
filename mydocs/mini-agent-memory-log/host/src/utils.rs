//! # Shared Utilities
//!
//! Common helper functions used across modules.

/// Safely truncate a string to at most `max_bytes` bytes without splitting
/// a multi-byte UTF-8 character. Returns the truncated slice with "..." appended
/// if truncation occurred.
///
/// This is critical for handling CJK characters (Chinese, Japanese, Korean)
/// which are 3 bytes each in UTF-8. Naive slicing like `&s[..60]` can panic
/// if the byte offset lands in the middle of a multi-byte character.
pub fn truncate_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Walk backwards from max_bytes to find a valid char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_ascii() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hello...");
    }

    #[test]
    fn test_truncate_cjk() {
        // Each CJK character is 3 bytes in UTF-8
        let s = "你好世界测试";
        // 6 chars × 3 bytes = 18 bytes total
        assert_eq!(s.len(), 18);
        // Truncate at 7 bytes: should back up to 6 (2 full chars)
        let result = truncate_str(s, 7);
        assert_eq!(result, "你好...");
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(truncate_str("", 10), "");
    }
}
