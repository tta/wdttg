use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct TextEditor {
    buffer: String,
    cursor: usize,
}

impl TextEditor {
    pub fn empty() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
        }
    }

    pub fn new(initial: &str) -> Self {
        let cursor = initial.len();
        Self {
            buffer: initial.to_string(),
            cursor,
        }
    }

    pub fn text(&self) -> &str {
        &self.buffer
    }

    pub fn into_string(self) -> String {
        self.buffer
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn set_text(&mut self, text: &str) {
        self.buffer = text.to_string();
        self.cursor = self.buffer.len();
    }

    pub fn cursor_line(&self) -> usize {
        self.buffer[..self.cursor].matches('\n').count()
    }

    pub fn cursor_col(&self) -> usize {
        let line_start = self.buffer[..self.cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
        self.buffer[line_start..self.cursor].chars().count()
    }

    fn line_start(&self) -> usize {
        self.buffer[..self.cursor].rfind('\n').map(|p| p + 1).unwrap_or(0)
    }

    fn line_end(&self) -> usize {
        self.buffer[self.cursor..]
            .find('\n')
            .map(|p| self.cursor + p)
            .unwrap_or(self.buffer.len())
    }

    fn prev_char_boundary(&self) -> usize {
        if self.cursor == 0 {
            return 0;
        }
        let mut i = self.cursor - 1;
        while i > 0 && !self.buffer.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    fn next_char_boundary(&self) -> usize {
        if self.cursor >= self.buffer.len() {
            return self.cursor;
        }
        let mut i = self.cursor + 1;
        while i < self.buffer.len() && !self.buffer.is_char_boundary(i) {
            i += 1;
        }
        i
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                self.cursor = self.line_start();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                self.cursor = self.line_end();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('h')) => {
                self.cursor = self.prev_char_boundary();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                self.cursor = self.next_char_boundary();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                self.delete_prev_word();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                let ls = self.line_start();
                self.buffer.drain(ls..self.cursor);
                self.cursor = ls;
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                let le = self.line_end();
                self.buffer.drain(self.cursor..le);
                true
            }
            (KeyModifiers::NONE, KeyCode::Left) => {
                self.cursor = self.prev_char_boundary();
                true
            }
            (KeyModifiers::NONE, KeyCode::Right) => {
                self.cursor = self.next_char_boundary();
                true
            }
            (KeyModifiers::NONE, KeyCode::Up) => {
                self.move_vertical(-1);
                true
            }
            (KeyModifiers::NONE, KeyCode::Down) => {
                self.move_vertical(1);
                true
            }
            (KeyModifiers::NONE, KeyCode::Home) => {
                self.cursor = self.line_start();
                true
            }
            (KeyModifiers::NONE, KeyCode::End) => {
                self.cursor = self.line_end();
                true
            }
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                if self.cursor > 0 {
                    let p = self.prev_char_boundary();
                    self.buffer.drain(p..self.cursor);
                    self.cursor = p;
                }
                true
            }
            (KeyModifiers::NONE, KeyCode::Delete) => {
                if self.cursor < self.buffer.len() {
                    let n = self.next_char_boundary();
                    self.buffer.drain(self.cursor..n);
                }
                true
            }
            (KeyModifiers::NONE, KeyCode::Enter) => {
                self.buffer.insert(self.cursor, '\n');
                self.cursor += 1;
                true
            }
            (_, KeyCode::Char(c)) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.buffer.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                true
            }
            _ => false,
        }
    }

    fn delete_prev_word(&mut self) {
        let before = self.buffer[..self.cursor].trim_end_matches(|c: char| c.is_whitespace());
        if before.is_empty() {
            let ws_start = self.buffer[..self.cursor]
                .char_indices()
                .rev()
                .skip_while(|(_, c)| c.is_whitespace())
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.buffer.drain(ws_start..self.cursor);
            self.cursor = ws_start;
            return;
        }
        if let Some(pos) = before.rfind(|c: char| c.is_whitespace()) {
            let delete_from = pos + 1;
            self.buffer.drain(delete_from..self.cursor);
            self.cursor = delete_from;
        } else {
            self.buffer.drain(0..self.cursor);
            self.cursor = 0;
        }
    }

    fn move_vertical(&mut self, delta: i32) {
        let col = self.cursor_col();

        if delta < 0 {
            let line_start = self.line_start();
            if line_start == 0 {
                self.cursor = 0;
                return;
            }
            let prev_line_end = line_start - 1;
            let prev_line_start = self.buffer[..prev_line_end]
                .rfind('\n')
                .map(|p| p + 1)
                .unwrap_or(0);
            let prev_line = &self.buffer[prev_line_start..prev_line_end];
            let prev_line_len = prev_line.chars().count();
            let byte_offset = prev_line
                .char_indices()
                .nth(col.min(prev_line_len))
                .map(|(i, _)| i)
                .unwrap_or(prev_line.len());
            self.cursor = prev_line_start + byte_offset;
        } else {
            let le = self.line_end();
            if le >= self.buffer.len() {
                return;
            }
            let next_line_start = le + 1;
            let next_line_end = self.buffer[next_line_start..]
                .find('\n')
                .map(|p| next_line_start + p)
                .unwrap_or(self.buffer.len());
            let next_line = &self.buffer[next_line_start..next_line_end];
            let next_line_len = next_line.chars().count();
            let byte_offset = next_line
                .char_indices()
                .nth(col.min(next_line_len))
                .map(|(i, _)| i)
                .unwrap_or(next_line.len());
            self.cursor = next_line_start + byte_offset;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_at_cursor() {
        let mut ed = TextEditor::empty();
        ed.handle_key(key('h'));
        ed.handle_key(key('i'));
        assert_eq!(ed.text(), "hi");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn test_left_right_movement() {
        let mut ed = TextEditor::new("hello");
        ed.handle_key(left());
        ed.handle_key(left());
        assert_eq!(ed.cursor_col(), 3);
        ed.handle_key(key('X'));
        assert_eq!(ed.text(), "helXlo");
    }

    #[test]
    fn test_ctrl_h_ctrl_l() {
        let mut ed = TextEditor::new("hello");
        ed.handle_key(ctrl('h'));
        ed.handle_key(ctrl('h'));
        assert_eq!(ed.cursor(), 3);
        ed.handle_key(ctrl('l'));
        assert_eq!(ed.cursor(), 4);
    }

    #[test]
    fn test_ctrl_w_delete_word() {
        let mut ed = TextEditor::new("hello world");
        ed.handle_key(ctrl('w'));
        assert_eq!(ed.text(), "hello ");
    }

    #[test]
    fn test_ctrl_w_mid_buffer() {
        let mut ed = TextEditor::new("hello world foo");
        ed.handle_key(left());
        ed.handle_key(left());
        ed.handle_key(left());
        ed.handle_key(ctrl('w'));
        assert_eq!(ed.text(), "hello foo");
    }

    #[test]
    fn test_ctrl_u_delete_to_line_start() {
        let mut ed = TextEditor::new("hello world");
        ed.handle_key(left());
        ed.handle_key(left());
        ed.handle_key(ctrl('u'));
        assert_eq!(ed.text(), "ld");
    }

    #[test]
    fn test_ctrl_u_multiline() {
        let mut ed = TextEditor::new("line one\nline two");
        for _ in 0..3 {
            ed.handle_key(left());
        }
        ed.handle_key(ctrl('u'));
        assert_eq!(ed.text(), "line one\ntwo");
    }

    #[test]
    fn test_ctrl_k_delete_to_line_end() {
        let mut ed = TextEditor::new("hello world");
        for _ in 0..5 {
            ed.handle_key(left());
        }
        ed.handle_key(ctrl('k'));
        assert_eq!(ed.text(), "hello ");
    }

    #[test]
    fn test_ctrl_k_multiline() {
        let mut ed = TextEditor::new("line one\nline two");
        for _ in 0..14 {
            ed.handle_key(left());
        }
        ed.handle_key(ctrl('k'));
        assert_eq!(ed.text(), "lin\nline two");
    }

    #[test]
    fn test_multiline_cursor_up_down() {
        let mut ed = TextEditor::new("hello\nworld");
        ed.handle_key(up());
        assert_eq!(ed.cursor_line(), 0);
        assert_eq!(ed.cursor_col(), 5);
        ed.handle_key(down());
        assert_eq!(ed.cursor_line(), 1);
    }

    #[test]
    fn test_enter_inserts_newline() {
        let mut ed = TextEditor::new("hello");
        for _ in 0..2 {
            ed.handle_key(left());
        }
        ed.handle_key(enter());
        assert_eq!(ed.text(), "hel\nlo");
    }

    #[test]
    fn test_backspace() {
        let mut ed = TextEditor::new("hello");
        ed.handle_key(backspace());
        assert_eq!(ed.text(), "hell");
    }

    #[test]
    fn test_ctrl_a_ctrl_e() {
        let mut ed = TextEditor::new("hello world");
        ed.handle_key(ctrl('a'));
        assert_eq!(ed.cursor_col(), 0);
        ed.handle_key(ctrl('e'));
        assert_eq!(ed.cursor_col(), 11);
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn left() -> KeyEvent {
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
    }

    fn up() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    }

    fn down() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    }

    fn backspace() -> KeyEvent {
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
}
