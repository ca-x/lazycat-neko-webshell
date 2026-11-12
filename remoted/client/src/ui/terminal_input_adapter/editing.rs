//! Bounded committed history for an asynchronous native input.
//!
//! `Input::value` is edit-count protected, but has no acknowledgement. An
//! app-owned generation header identifies rollovers without guessing from the
//! user's text. Every request differs, so NSH's prop diff can retry stale writes.
use sha2::{Digest, Sha256};

pub(super) const HISTORY_LIMIT: usize = 1024;
pub(super) const RETAINED_BYTES: usize = 128;
const START: &str = "\u{2060}";
const END: &str = "\u{2063}";
pub(super) const HEADER_BYTES: usize = START.len() + 16 + END.len();

fn header(generation: u64) -> String {
    format!("{START}{generation:016x}{END}")
}

pub(super) fn native_parts(text: &str) -> (Option<u64>, &str) {
    if !text.starts_with(START) {
        return (None, text);
    }
    if let Some(generation) = text
        .get(..HEADER_BYTES)
        .and_then(|header| header.strip_prefix(START))
        .and_then(|header| header.strip_suffix(END))
        && let Ok(generation) = u64::from_str_radix(generation, 16)
    {
        return (Some(generation), &text[HEADER_BYTES..]);
    }
    // Backspace can remove the end of an otherwise empty header. Its remaining
    // metadata is never user input and must not be retained or sent to the PTY.
    (None, "")
}

struct Baseline {
    bytes: usize,
    digest: [u8; 32],
}
impl Baseline {
    fn new(text: &str) -> Self {
        Self {
            bytes: text.len(),
            digest: Sha256::digest(text.as_bytes()).into(),
        }
    }
    fn added<'a>(&self, text: &'a str) -> &'a str {
        text.split_at_checked(self.bytes)
            .filter(|(prefix, _)| {
                self.digest == <[u8; 32]>::from(Sha256::digest(prefix.as_bytes()))
            })
            .map_or("", |(_, added)| added)
    }
}
impl Default for Baseline {
    fn default() -> Self {
        Self::new("")
    }
}

struct Rollover {
    generation: u64,
    baseline: Baseline,
}

pub(super) struct Editing {
    source: Option<u64>,
    generation: u64,
    baseline: Baseline,
    rollover: Option<Rollover>,
    value: Option<String>,
    pending: Option<String>,
    composing: bool,
}
impl Default for Editing {
    fn default() -> Self {
        Self {
            source: Some(0),
            generation: 0,
            baseline: Baseline::default(),
            rollover: None,
            value: None,
            pending: None,
            composing: false,
        }
    }
}
impl Editing {
    pub fn initial_value() -> String {
        header(0)
    }
    pub fn take_value(&mut self) -> Option<String> {
        self.value.take()
    }
    pub fn reset(&mut self) -> String {
        self.pending = None;
        self.composing = false;
        self.rollover("");
        self.source = Some(self.generation);
        self.baseline = Baseline::default();
        self.rollover = None;
        self.take_value().expect("reset requested a native value")
    }
    pub fn change(&mut self, text: &str) -> String {
        if self.composing {
            self.pending = Some(text.to_owned());
            String::new()
        } else {
            self.commit(text)
        }
    }
    pub fn composition(&mut self, composing: bool) -> String {
        if composing {
            self.composing = true;
            return String::new();
        }
        if !std::mem::take(&mut self.composing) {
            return String::new();
        }
        self.pending
            .take()
            .map_or_else(String::new, |text| self.commit(&text))
    }
    pub fn submit(&mut self, text: &str) -> String {
        let added = self.commit(text);
        self.rollover("");
        added
    }
    fn commit(&mut self, text: &str) -> String {
        let (source, body) = native_parts(text);
        if source != self.source {
            if self
                .rollover
                .as_ref()
                .is_some_and(|next| Some(next.generation) == source)
            {
                let next = self.rollover.take().expect("checked rollover");
                self.source = Some(next.generation);
                self.baseline = next.baseline;
            } else if source.is_none() {
                // A software Backspace can remove the hidden header. This is
                // a deletion/replacement, not new PTY text; re-arm the sink.
                self.source = None;
                self.baseline = Baseline::new(body);
                self.rollover(tail(body));
                return String::new();
            } else {
                return String::new();
            }
        }
        // Deletions have key events. Replacements of submitted text must not
        // synthesize remote edits. A digest retains no unbounded text history.
        let added = self.baseline.added(body).to_owned();
        self.baseline = Baseline::new(body);
        if body.len() >= HISTORY_LIMIT || source.is_none() {
            self.rollover(tail(body));
        }
        added
    }
    fn rollover(&mut self, retained: &str) {
        self.generation = self.generation.wrapping_add(1);
        self.rollover = Some(Rollover {
            generation: self.generation,
            baseline: Baseline::new(retained),
        });
        self.value = Some(format!("{}{retained}", header(self.generation)));
    }
}
fn tail(text: &str) -> &str {
    let mut start = text.len().saturating_sub(RETAINED_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
