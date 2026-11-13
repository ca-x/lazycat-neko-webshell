//! Bounded query-only view of Kitty APCs for a server PTY reply authority.
//! Raw PTY output must be broadcast unchanged; this filter is only for the
//! reply parser so it never retains image payloads or opens their local paths.
use std::borrow::Cow;

/// Streaming query-only copy for a reply parser, separate from published output.
#[derive(Default)]
pub struct ReplyOnlyInputFilter {
    state: ReplyOnlyFilterState,
    kitty_graphics: Vec<u8>,
    kitty_graphics_overflowed: bool,
}

/// Maximum retained control sequence, including the query payload and terminator.
pub const MAX_KITTY_QUERY_BYTES: usize = 4096;

#[derive(Default)]
enum ReplyOnlyFilterState {
    #[default]
    Normal,
    Escape,
    ApplicationProgramCommand {
        c1: bool,
    },
    KittyGraphics {
        c1: bool,
        escape: bool,
    },
}

impl ReplyOnlyInputFilter {
    /// Keep standard VT bytes and bounded Kitty queries; discard image mutations.
    pub fn filter<'a>(&mut self, input: &'a [u8]) -> Cow<'a, [u8]> {
        const ESC: u8 = 0x1b;
        const APC: u8 = 0x9f;
        const ST: u8 = 0x9c;

        if matches!(self.state, ReplyOnlyFilterState::Normal)
            && !input.ends_with(&[ESC])
            && !input.contains(&APC)
            && !input.windows(2).any(|bytes| bytes == b"\x1b_")
        {
            return Cow::Borrowed(input);
        }

        let mut output = Vec::with_capacity(input.len());
        let mut index = 0;
        while index < input.len() {
            let byte = input[index];
            match self.state {
                ReplyOnlyFilterState::Normal => match byte {
                    ESC => self.state = ReplyOnlyFilterState::Escape,
                    APC => {
                        self.state = ReplyOnlyFilterState::ApplicationProgramCommand { c1: true };
                    }
                    _ => output.push(byte),
                },
                ReplyOnlyFilterState::Escape => {
                    if byte == b'_' {
                        self.state = ReplyOnlyFilterState::ApplicationProgramCommand { c1: false };
                    } else {
                        output.push(ESC);
                        self.state = ReplyOnlyFilterState::Normal;
                        continue;
                    }
                }
                ReplyOnlyFilterState::ApplicationProgramCommand { c1 } => {
                    if byte == b'G' {
                        self.kitty_graphics.clear();
                        self.kitty_graphics_overflowed = false;
                        if c1 {
                            self.kitty_graphics.extend_from_slice(&[APC, b'G']);
                        } else {
                            self.kitty_graphics.extend_from_slice(b"\x1b_G");
                        }
                        self.state = ReplyOnlyFilterState::KittyGraphics { c1, escape: false };
                    } else {
                        if c1 {
                            output.push(APC);
                        } else {
                            output.extend_from_slice(b"\x1b_");
                        }
                        self.state = ReplyOnlyFilterState::Normal;
                        continue;
                    }
                }
                ReplyOnlyFilterState::KittyGraphics { c1, escape } => {
                    self.capture_kitty_graphics_byte(byte);
                    if byte == ST || (escape && byte == b'\\') {
                        if !self.kitty_graphics_overflowed
                            && kitty_graphics_is_query(&self.kitty_graphics)
                        {
                            if c1 {
                                output.extend_from_slice(b"\x1b_G");
                                let command = &self.kitty_graphics[2..];
                                let command = command
                                    .strip_suffix(&[ST])
                                    .or_else(|| command.strip_suffix(b"\x1b\\"))
                                    .unwrap_or(command);
                                output.extend_from_slice(command);
                                output.extend_from_slice(b"\x1b\\");
                            } else {
                                output.extend_from_slice(&self.kitty_graphics);
                            }
                        }
                        self.kitty_graphics.clear();
                        self.kitty_graphics_overflowed = false;
                        self.state = ReplyOnlyFilterState::Normal;
                    } else {
                        self.state = ReplyOnlyFilterState::KittyGraphics {
                            c1,
                            escape: byte == ESC,
                        };
                    }
                }
            }
            index += 1;
        }
        Cow::Owned(output)
    }

    fn capture_kitty_graphics_byte(&mut self, byte: u8) {
        if self.kitty_graphics_overflowed {
            return;
        }
        if self.kitty_graphics.len() >= MAX_KITTY_QUERY_BYTES {
            self.kitty_graphics.clear();
            self.kitty_graphics_overflowed = true;
            return;
        }
        self.kitty_graphics.push(byte);
    }
}

fn kitty_graphics_is_query(sequence: &[u8]) -> bool {
    const ESC: u8 = 0x1b;
    const APC: u8 = 0x9f;
    const ST: u8 = 0x9c;

    let body = if sequence.starts_with(b"\x1b_G") {
        &sequence[3..]
    } else if sequence.starts_with(&[APC, b'G']) {
        &sequence[2..]
    } else {
        return false;
    };
    let body = if body.ends_with(&[ESC, b'\\']) {
        &body[..body.len() - 2]
    } else if body.ends_with(&[ST]) {
        &body[..body.len() - 1]
    } else {
        return false;
    };
    let control = body
        .iter()
        .position(|byte| *byte == b';')
        .map_or(body, |separator| &body[..separator]);
    let mut query_action = false;
    for field in control.split(|byte| *byte == b',') {
        if !field.starts_with(b"a=") {
            continue;
        }
        if query_action || field != b"a=q" {
            return false;
        }
        query_action = true;
    }
    query_action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_queries_survive_but_large_image_transmissions_never_reach_the_reply_vt() {
        let query = b"\x1b_Ga=q,i=31,f=24,s=1,v=1;AAAA\x1b\\";
        for split in 0..=query.len() {
            let mut filter = ReplyOnlyInputFilter::default();
            let mut output = filter.filter(&query[..split]).into_owned();
            output.extend_from_slice(&filter.filter(&query[split..]));
            assert_eq!(output, query);
        }
        let mut filter = ReplyOnlyInputFilter::default();
        assert_eq!(&*filter.filter(b"before\x1b_Ga=T,f=100,m=1;"), b"before");
        let payload = [b'A'; 4096];
        for _ in 0..1024 {
            assert!(filter.filter(&payload).is_empty());
            assert!(filter.kitty_graphics.len() <= MAX_KITTY_QUERY_BYTES);
        }
        assert_eq!(&*filter.filter(b"\x1b\\after"), b"after");
        assert_eq!(&*filter.filter(query), query);
    }

    #[test]
    fn ambiguous_actions_are_not_queries_and_standard_vt_bytes_are_preserved() {
        let mut filter = ReplyOnlyInputFilter::default();
        assert!(filter.filter(b"\x1b_Ga=q,a=T,i=31;AAAA\x1b\\").is_empty());
        let text = "猫\x1b[31mRED\x1b[0m\r\n".as_bytes();
        assert_eq!(&*filter.filter(text), text);
    }
}
