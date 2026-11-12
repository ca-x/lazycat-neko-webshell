//! Herdr pane.read returns replacement plain text, not a PTY byte stream.
//! Normalize LF and bound snapshots without wrapping a long source line silently.
use nsh_base::TerminalSize;
use unicode_width::UnicodeWidthChar;

const MAX_COLUMNS: usize = 1000;
const MAX_ROWS: usize = 1000;

#[derive(Default)]
pub struct ScreenText {
    cr: bool,
    column: usize,
    width: usize,
    row: usize,
    pub clipped: bool,
}
impl ScreenText {
    pub fn push(&mut self, text: &str) -> (String, TerminalSize) {
        let mut output = String::with_capacity(text.len());
        for character in text.chars() {
            if self.row >= MAX_ROWS {
                self.clipped = true;
                continue;
            }
            match character {
                '\n' => {
                    self.column = 0;
                    self.row += 1;
                    if self.row >= MAX_ROWS {
                        // An extra LF would scroll the first source row away.
                        self.clipped = true;
                        continue;
                    }
                    if !self.cr {
                        output.push('\r');
                    }
                    output.push('\n');
                }
                '\r' => {
                    self.column = 0;
                    output.push('\r');
                }
                '\t' => {
                    let end = (self.column / 8 + 1) * 8;
                    output.extend(std::iter::repeat_n(
                        ' ',
                        end.min(MAX_COLUMNS).saturating_sub(self.column),
                    ));
                    self.column = end;
                    self.clipped |= end > MAX_COLUMNS;
                }
                _ => {
                    let end = self.column.saturating_add(character.width().unwrap_or(0));
                    if end <= MAX_COLUMNS {
                        output.push(character);
                    } else {
                        self.clipped = true;
                    }
                    self.column = end;
                }
            }
            self.width = self.width.max(self.column.min(MAX_COLUMNS));
            self.cr = character == '\r';
        }
        (
            output,
            TerminalSize::new(
                self.width.max(80) as u16,
                (self.row + 1).clamp(24, MAX_ROWS) as u16,
            ),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_lines_start_at_column_zero_and_split_crlf_stays_single() {
        let mut screen = ScreenText::default();
        assert_eq!(screen.push("one\ntwo\r").0, "one\r\ntwo\r");
        assert_eq!(screen.push("\n猫\n").0, "\n猫\r\n");
        assert!(!screen.clipped);
    }
    #[test]
    fn source_geometry_includes_wide_text_and_long_screens_without_remote_resize() {
        let mut screen = ScreenText::default();
        let (_, size) = screen.push(&format!("{}\n{}", "猫".repeat(70), "line\n".repeat(50)));
        assert_eq!(size.cols, 140);
        assert_eq!(size.rows, 52);
    }
    #[test]
    fn oversized_source_line_is_clipped_explicitly_instead_of_wrapping() {
        let mut screen = ScreenText::default();
        let (text, size) = screen.push(&format!("{}\nlast", "猫".repeat(501)));
        assert!(screen.clipped);
        assert_eq!(text, format!("{}\r\nlast", "猫".repeat(500)));
        assert_eq!(size.cols, 1000);
    }
    #[test]
    fn oversized_snapshot_does_not_scroll_away_its_first_line() {
        let mut screen = ScreenText::default();
        let (text, size) = screen.push(&"row\n".repeat(1001));
        assert!(screen.clipped);
        assert_eq!(text.matches('\n').count(), 999);
        assert_eq!(size.rows, 1000);
    }
}
