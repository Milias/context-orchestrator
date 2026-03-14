/// Incrementally splits streaming text into visible content and think blocks.
/// Handles multiple `<think>...</think>` blocks and single-chunk edge cases.
pub(super) struct ThinkSplitter {
    visible: String,
    think_blocks: Vec<String>,
    buffer: String,
    in_think: bool,
}

impl ThinkSplitter {
    pub fn new() -> Self {
        Self {
            visible: String::new(),
            think_blocks: Vec::new(),
            buffer: String::new(),
            in_think: false,
        }
    }

    pub fn push(&mut self, chunk: &str) {
        self.buffer.push_str(chunk);
        self.drain_buffer();
    }

    fn drain_buffer(&mut self) {
        loop {
            if self.in_think {
                match self.buffer.find("</think>") {
                    Some(end) => {
                        self.think_blocks.push(self.buffer[..end].to_string());
                        self.buffer = self.buffer[end + 8..].to_string();
                        self.in_think = false;
                    }
                    None => break,
                }
            } else if let Some(start) = self.buffer.find("<think>") {
                self.visible.push_str(&self.buffer[..start]);
                self.buffer = self.buffer[start + 7..].to_string();
                self.in_think = true;
            } else {
                // Keep a tail that could be a partial `<think>` tag.
                // Walk back to a char boundary to avoid slicing inside multi-byte chars.
                let mut safe = self.buffer.len().saturating_sub(6);
                while safe > 0 && !self.buffer.is_char_boundary(safe) {
                    safe -= 1;
                }
                self.visible.push_str(&self.buffer[..safe]);
                self.buffer = self.buffer[safe..].to_string();
                break;
            }
        }
    }

    pub fn visible(&self) -> &str {
        &self.visible
    }

    pub fn is_thinking(&self) -> bool {
        self.in_think
    }

    /// Finalize: flush remaining buffer and return (visible, `think_content`).
    /// Leading whitespace is trimmed from visible output — it is an API
    /// formatting artifact (e.g. `\n` after `</think>` blocks), not content.
    pub fn finish(mut self) -> (String, String) {
        if self.in_think {
            // Unclosed think block — treat remaining buffer as think content
            self.think_blocks.push(std::mem::take(&mut self.buffer));
        } else {
            self.visible.push_str(&self.buffer);
        }
        let think = self.think_blocks.join("\n");
        let visible = self.visible.trim_start().to_string();
        (visible, think)
    }
}

#[cfg(test)]
#[path = "think_splitter_tests.rs"]
mod tests;
