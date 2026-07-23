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

    /// The half-open char range of the line the cursor sits on, excluding its terminating `\n`.
    fn line_bounds(&self, idx: usize) -> (usize, usize) {
        let start = self.chars[..idx]
            .iter()
            .rposition(|&c| c == '\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        let end = self.chars[idx..]
            .iter()
            .position(|&c| c == '\n')
            .map(|p| idx + p)
            .unwrap_or(self.chars.len());
        (start, end)
    }

    /// The cursor as `(row, column)`, both 0-based — what the view needs to place the terminal
    /// cursor on a soft-wrapped multi-line buffer.
    pub fn line_col(&self) -> (usize, usize) {
        let (start, _) = self.line_bounds(self.cursor);
        let row = self.chars[..start].iter().filter(|&&c| c == '\n').count();
        (row, self.cursor - start)
    }

    /// Whether the buffer holds a soft newline (Shift/Alt+Enter), so ↑/↓ have somewhere to go.
    pub fn is_multiline(&self) -> bool {
        self.chars.contains(&'\n')
    }

    /// Move the cursor up one line, keeping the column where the shorter line allows. Returns
    /// `false` when already on the first line — the caller then scrolls the chat instead (010).
    pub fn cursor_up(&mut self) -> bool {
        let (start, _) = self.line_bounds(self.cursor);
        if start == 0 {
            return false;
        }
        let col = self.cursor - start;
        let (prev_start, prev_end) = self.line_bounds(start - 1);
        self.cursor = prev_start + col.min(prev_end - prev_start);
        true
    }

    /// Move the cursor down one line. Returns `false` on the last line (the caller scrolls).
    pub fn cursor_down(&mut self) -> bool {
        let (start, end) = self.line_bounds(self.cursor);
        if end >= self.chars.len() {
            return false;
        }
        let col = self.cursor - start;
        let next_start = end + 1;
        let (_, next_end) = self.line_bounds(next_start);
        self.cursor = next_start + col.min(next_end - next_start);
        true
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

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(s: &str) -> InputState {
        let mut i = InputState::default();
        i.insert_str(s);
        i
    }

    #[test]
    fn the_cursor_moves_between_soft_newlines_and_reports_where_it_is() {
        let mut i = buffer("one\ntwo");
        assert!(i.is_multiline());
        assert_eq!(i.line_col(), (1, 3), "at the end of the second line");
        assert!(i.cursor_up());
        assert_eq!(i.line_col(), (0, 3), "same column, one line up");
        assert!(i.cursor_down());
        assert_eq!(i.line_col(), (1, 3));
    }

    #[test]
    fn the_outer_lines_refuse_to_move_so_the_caller_can_scroll_instead() {
        // The `false` return is the whole contract with `handle_input_key` (010): nowhere left to go
        // inside the buffer means the keystroke belongs to the chat flow.
        let mut single = buffer("just one line");
        assert!(!single.cursor_up());
        assert!(!single.cursor_down());
        assert_eq!(
            single.line_col(),
            (0, 13),
            "a refused move never shifts the cursor"
        );

        let mut multi = buffer("one\ntwo");
        assert!(multi.cursor_up());
        assert!(!multi.cursor_up(), "already on the first line");
        assert!(multi.cursor_down());
        assert!(!multi.cursor_down(), "already on the last line");
    }

    #[test]
    fn a_shorter_line_clamps_the_column_rather_than_overshooting() {
        let mut i = buffer("ab\nlonger");
        assert_eq!(i.line_col(), (1, 6));
        assert!(i.cursor_up());
        assert_eq!(i.line_col(), (0, 2), "clamped to the end of the short line");
    }

    #[test]
    fn an_empty_buffer_has_nowhere_to_go() {
        let mut i = InputState::default();
        assert!(!i.is_multiline());
        assert!(!i.cursor_up());
        assert!(!i.cursor_down());
        assert_eq!(i.line_col(), (0, 0));
    }
}
