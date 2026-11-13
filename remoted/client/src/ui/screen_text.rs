//! Herdr pane.read serializes replacement rows with ANSI styling, not raw PTY history.
//! Normalize LF and measure visible cells while preserving bounded escape sequences.
use nsh_base::TerminalSize;
use unicode_width::UnicodeWidthChar;

const MAX_COLUMNS: usize = 1000;
const MAX_ROWS: usize = 1000;
const MAX_ESCAPE_BYTES: usize = 4096;

#[derive(Default)]
enum Escape {
    #[default]
    None,
    Start,
    Intermediate,
    Csi,
    String {
        osc: bool,
        escaped: bool,
    },
}

#[derive(Default)]
pub struct ScreenText {
    cr: bool,
    column: usize,
    width: usize,
    row: usize,
    escape: Escape,
    pending: String,
    discarded: bool,
    pub clipped: bool,
}
impl ScreenText {
    pub fn push(&mut self, text: &str) -> (String, TerminalSize) {
        let mut output = String::with_capacity(text.len());
        for character in text.chars() {
            if self.control(character, &mut output) {
                continue;
            }
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
                '\u{8}' => {
                    self.column = self.column.saturating_sub(1);
                    output.push(character);
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

    // Current Herdr ANSI snapshots serialize positioned cells as rows/spaces and
    // use escapes for styling. Their controls must neither widen a row nor turn
    // an OSC payload into text when a transport chunk splits the sequence.
    fn control(&mut self, character: char, output: &mut String) -> bool {
        if matches!(self.escape, Escape::None) {
            match character {
                '\u{1b}' => {
                    self.escape = Escape::Start;
                    self.pending.push(character);
                }
                '\u{9b}' => {
                    self.escape = Escape::Csi;
                    self.pending.push_str("\x1b[");
                }
                '\u{9d}' => {
                    self.escape = Escape::String {
                        osc: true,
                        escaped: false,
                    };
                    self.pending.push_str("\x1b]");
                }
                _ => return false,
            }
            return true;
        }
        if !self.discarded {
            if self.pending.len() + character.len_utf8() <= MAX_ESCAPE_BYTES {
                self.pending.push(character);
            } else {
                self.pending.clear();
                self.discarded = true;
                self.clipped = true;
            }
        }
        let complete = match self.escape {
            Escape::Start => {
                self.escape = match character {
                    '[' => Escape::Csi,
                    ']' => Escape::String {
                        osc: true,
                        escaped: false,
                    },
                    'P' | 'X' | '^' | '_' => Escape::String {
                        osc: false,
                        escaped: false,
                    },
                    '\u{20}'..='\u{2f}' => Escape::Intermediate,
                    _ => Escape::None,
                };
                matches!(self.escape, Escape::None)
            }
            Escape::Intermediate => ('\u{30}'..='\u{7e}').contains(&character),
            Escape::Csi => ('\u{40}'..='\u{7e}').contains(&character),
            Escape::String { osc, escaped } => {
                self.escape = Escape::String {
                    osc,
                    escaped: character == '\u{1b}',
                };
                (osc && character == '\u{7}')
                    || character == '\u{9c}'
                    || (escaped && character == '\\')
            }
            Escape::None => unreachable!("an escape was started above"),
        };
        if complete {
            if !self.discarded {
                output.push_str(&self.pending);
            }
            self.pending.clear();
            self.discarded = false;
            self.escape = Escape::None;
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ansi_snapshot_geometry_counts_cells_not_sgr_or_osc_payloads() {
        let source = format!(
            "\x1b]0;{}\x07\x1b[38;2;12;34;56m{}\x1b[0m\nlast",
            "title".repeat(100),
            "猫".repeat(60)
        );
        let mut screen = ScreenText::default();
        let (output, size) = screen.push(&source);
        assert_eq!(output, source.replace('\n', "\r\n"));
        assert_eq!(size.cols, 120);
        assert_eq!(size.rows, 24);
        assert!(!screen.clipped);
    }
    #[test]
    fn split_csi_osc_and_st_sequences_keep_styles_without_leaking_payload_into_geometry() {
        let source = format!(
            "\x1b]8;;https://example.test/link\x1b\\\x1b[38;5;1m{}\x1b[0m\x1b]8;;\x1b\\\r\nend",
            "猫".repeat(50)
        );
        let mut screen = ScreenText::default();
        let mut output = String::new();
        let mut size = TerminalSize::default();
        for character in source.chars() {
            let (chunk, current) = screen.push(&character.to_string());
            output.push_str(&chunk);
            size = current;
            assert!(screen.pending.len() <= MAX_ESCAPE_BYTES);
        }
        assert_eq!(output, source);
        assert_eq!(size.cols, 100);
        assert!(!screen.clipped);
    }
    #[test]
    fn oversized_controls_are_bounded_discarded_and_never_become_visible_text() {
        for (prefix, suffix) in [("\x1b[", "m"), ("\x1b]0;", "\x1b\\")] {
            let mut screen = ScreenText::default();
            let (first, _) = screen.push(&format!("{prefix}{}", "0".repeat(MAX_ESCAPE_BYTES * 2)));
            assert!(first.is_empty());
            assert!(screen.pending.len() <= MAX_ESCAPE_BYTES);
            let (last, size) = screen.push(&format!("{suffix}ok"));
            assert_eq!(last, "ok");
            assert_eq!(size.cols, 80);
            assert!(screen.clipped);
        }
    }
    #[test]
    fn a_full_width_styled_source_line_does_not_clip_escape_bytes_as_columns() {
        let source = "\x1b[31mx\x1b[0m".repeat(MAX_COLUMNS);
        let mut screen = ScreenText::default();
        let (output, size) = screen.push(&source);
        assert_eq!(output, source);
        assert_eq!(size.cols, MAX_COLUMNS as u16);
        assert!(!screen.clipped);
    }
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
