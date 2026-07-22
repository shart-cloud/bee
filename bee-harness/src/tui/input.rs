//! Minimal multi-line text input (008-grid-tui, US1 T014).
//!
//! A `Vec<char>` buffer + cursor (char-indexed, so insert/delete never split a UTF-8 boundary), with
//! input history. Pure state — no terminal, no rendering here (the view draws `text()` + `cursor()`).

/// The editable input line(s) below the chat.
#[derive(Debug, Default)]
pub struct InputState {
    chars: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// Position while browsing history with ↑/↓ (`None` = editing a fresh line).
    hist: Option<usize>,
}

impl InputState {
    /// The current buffer as a string (may contain `\n`).
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// Char-index of the cursor (0..=len).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Insert one character at the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
        self.hist = None;
    }

    /// Insert a string at the cursor (multi-line paste safe).
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
        self.hist = None;
    }

    /// Insert a newline (Shift/Alt+Enter — a soft break, not a submit).
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    /// Delete the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
    }
    pub fn home(&mut self) {
        self.cursor = 0;
    }
    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }

    /// Clear the buffer (does not touch history).
    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
        self.hist = None;
    }

    /// Take the buffer for submission: `None` when it's blank, otherwise the text (recorded in
    /// history and cleared). The caller sends the returned text to the model.
    pub fn take(&mut self) -> Option<String> {
        let text = self.text();
        if text.trim().is_empty() {
            self.clear();
            return None;
        }
        self.history.push(text.clone());
        self.clear();
        Some(text)
    }

    /// Recall the previous history entry into the buffer (↑).
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.hist {
            None => self.history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist = Some(next);
        self.set_from(self.history[next].clone());
    }

    /// Move forward through history toward the fresh line (↓).
    pub fn history_next(&mut self) {
        match self.hist {
            Some(i) if i + 1 < self.history.len() => {
                self.hist = Some(i + 1);
                self.set_from(self.history[i + 1].clone());
            }
            Some(_) => {
                // Past the newest entry → back to an empty fresh line.
                self.hist = None;
                self.clear();
            }
            None => {}
        }
    }

    fn set_from(&mut self, s: String) {
        self.chars = s.chars().collect();
        self.cursor = self.chars.len();
    }
}
