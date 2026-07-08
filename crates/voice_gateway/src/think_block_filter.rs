//! Streaming filter that removes `<think>...</think>` and `<thinking>...</thinking>`
//! reasoning blocks from a visible text stream, *including* the content between the
//! tags.
//!
//! This is the only text-channel reasoning compat the gateway performs. Pi itself
//! emits reasoning as separate `thinking_*` events which the mapper ignores by
//! construction; this filter handles the residual misconfigured-endpoint case where
//! reasoning is injected directly into the visible text channel as `<think>` tags.

const OPEN_TAGS: [&str; 2] = ["<thinking>", "<think>"];
const CLOSE_TAGS: [&str; 2] = ["</thinking>", "</think>"];

/// Streaming remover for `<think>...</think>` / `<thinking>...</thinking>` blocks.
#[derive(Default)]
pub struct ThinkBlockFilter {
    in_block: bool,
    pending: String,
}

impl ThinkBlockFilter {
    pub fn new() -> Self {
        Self {
            in_block: false,
            pending: String::new(),
        }
    }

    /// Feed one streamed chunk; returns the visible text to emit for this chunk.
    /// Tags split across chunks are held back and resolved across calls, so a block
    /// never leaks regardless of how the stream is chunked.
    pub fn filter(&mut self, chunk: &str) -> String {
        let mut buf = std::mem::take(&mut self.pending);
        buf.push_str(chunk);
        let chars: Vec<char> = buf.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '<' {
                let rest: String = chars[i..].iter().collect();
                let tags = if self.in_block { CLOSE_TAGS } else { OPEN_TAGS };
                if let Some(len) = match_full(&rest, &tags) {
                    self.in_block = !self.in_block;
                    i += len;
                    continue;
                }
                if let Some(prefix) = partial_prefix(&rest, &tags) {
                    // Incomplete tag (possibly split across chunks): hold it back.
                    self.pending = prefix;
                    break;
                }
            }
            if !self.in_block {
                out.push(chars[i]);
            }
            i += 1;
        }
        out
    }
}

/// If `rest` begins with one of `tags`, return that tag's length.
fn match_full(rest: &str, tags: &[&str]) -> Option<usize> {
    for tag in tags {
        if rest.starts_with(tag) {
            return Some(tag.len());
        }
    }
    None
}

/// If `rest` is a proper prefix of one of `tags` (an incomplete tag), return it so
/// the caller can defer it to the next chunk.
fn partial_prefix(rest: &str, tags: &[&str]) -> Option<String> {
    for tag in tags {
        if tag.starts_with(rest) && rest.len() < tag.len() {
            return Some(rest.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_clean_text_through_unchanged() {
        let mut f = ThinkBlockFilter::new();
        assert_eq!(f.filter("Loud and clear, Snake."), "Loud and clear, Snake.");
    }

    #[test]
    fn strips_complete_think_block_including_content_in_one_chunk() {
        let mut f = ThinkBlockFilter::new();
        let out = f.filter("<think>let me reason</think>The answer is 42.");
        assert_eq!(out, "The answer is 42.");
    }

    #[test]
    fn strips_think_block_split_across_chunks() {
        let mut f = ThinkBlockFilter::new();
        let out1 = f.filter("<think");
        let out2 = f.filter(">secret reasoning</think>The answer is 42.");
        assert_eq!(format!("{out1}{out2}"), "The answer is 42.");
    }

    #[test]
    fn strips_longer_thinking_variant_including_content() {
        let mut f = ThinkBlockFilter::new();
        let out = f.filter("Before. <thinking>hidden</thinking> After.");
        assert_eq!(out, "Before.  After.");
    }

    #[test]
    fn strips_multiple_blocks_with_split_closing_tag() {
        let mut f = ThinkBlockFilter::new();
        let out1 = f.filter("A<think>x</think>B<think>y</think");
        let out2 = f.filter(">C");
        assert_eq!(format!("{out1}{out2}"), "ABC");
    }
}
