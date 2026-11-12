//! Rendering is never a second PTY reply authority, including during replay.
use neko_ghostty::{Emulator, Error, Event, StyleColor, Underline};
use nsh_base::{
    TerminalColor, TerminalCursor, TerminalEmulator, TerminalEmulatorEvent,
    TerminalEmulatorFactory, TerminalLine, TerminalModes, TerminalRun, TerminalSize, TerminalStyle,
};
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub operation: &'static str,
    pub message: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Ghostty stopped ({}): {}",
            self.operation, self.message
        )
    }
}

struct State {
    kernel: Option<Emulator>,
    failure: Option<Failure>,
    failure_pending: bool,
    lines: BTreeMap<usize, TerminalLine>,
    count: usize,
    modes: TerminalModes,
}

impl State {
    fn fail(&mut self, operation: &'static str, error: impl ToString) {
        if self.failure.is_none() {
            self.failure = Some(Failure {
                operation,
                message: error.to_string(),
            });
            self.failure_pending = true;
        }
    }

    fn query<Value>(
        &mut self,
        operation: &'static str,
        query: impl FnOnce(&mut Emulator) -> Result<Value, Error>,
    ) -> Option<Value> {
        if self.failure.is_some() {
            return None;
        }
        match query(self.kernel.as_mut()?) {
            Ok(value) => Some(value),
            Err(error) => {
                self.fail(operation, error);
                None
            }
        }
    }

    fn error_line(&self) -> TerminalLine {
        let message = self
            .failure
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let mut style = TerminalStyle::default();
        style.bold = true;
        TerminalLine::new(vec![TerminalRun::new(message, style)])
    }
}

pub struct GhosttyEmulator(RefCell<State>);

impl GhosttyEmulator {
    pub fn new(size: TerminalSize, scrollback: usize) -> Result<Self, Error> {
        let kernel = Emulator::new(size.cols, size.rows, scrollback)?;
        Ok(Self::from_kernel(size, Some(kernel)))
    }

    fn from_kernel(size: TerminalSize, kernel: Option<Emulator>) -> Self {
        Self(RefCell::new(State {
            kernel,
            failure: None,
            failure_pending: false,
            lines: BTreeMap::new(),
            count: usize::from(size.rows),
            modes: TerminalModes::default(),
        }))
    }

    pub fn failed(size: TerminalSize, error: Error) -> Self {
        let emulator = Self::from_kernel(size, None);
        emulator.0.borrow_mut().fail("initialize", error);
        emulator
    }

    pub fn failure(&self) -> Option<Failure> {
        self.0.borrow().failure.clone()
    }
}

fn color(color: StyleColor) -> Option<TerminalColor> {
    match color {
        StyleColor::None => None,
        StyleColor::Palette(index) => Some(TerminalColor::Indexed(index.0)),
        StyleColor::Rgb(color) => Some(TerminalColor::Rgb(color.r, color.g, color.b)),
    }
}

fn line(runs: Vec<neko_ghostty::Run>) -> TerminalLine {
    TerminalLine::new(
        runs.into_iter()
            .map(|run| {
                let mut style = TerminalStyle::default();
                style.fg = color(run.style.fg_color);
                style.bg = color(run.style.bg_color);
                style.bold = run.style.bold;
                style.dim = run.style.faint;
                style.italic = run.style.italic;
                style.underline = run.style.underline != Underline::None;
                style.strikethrough = run.style.strikethrough;
                style.inverse = run.style.inverse;
                style.hidden = run.style.invisible;
                TerminalRun::new(run.text, style)
            })
            .collect(),
    )
}

impl TerminalEmulator for GhosttyEmulator {
    fn feed(&mut self, bytes: &[u8]) {
        let state = self.0.get_mut();
        if state.query("feed", |kernel| {
            kernel.feed(bytes);
            kernel.processing_error()
        }) == Some(true)
        {
            state.fail("feed", "VT processing failed");
        }
    }
    fn resize(&mut self, size: TerminalSize) {
        let state = self.0.get_mut();
        state.query("resize", |kernel| kernel.resize(size.cols, size.rows));
    }
    fn line_count(&self) -> usize {
        let mut state = self.0.borrow_mut();
        if let Some(count) = state.query("line count", |kernel| kernel.line_count()) {
            state.count = count;
            state.lines.retain(|index, _| *index < count);
        }
        state
            .count
            .saturating_add(usize::from(state.failure.is_some()))
    }
    fn line(&self, index: usize) -> TerminalLine {
        let mut state = self.0.borrow_mut();
        if let Some(runs) = state.query("render line", |kernel| kernel.line(index)) {
            let line = line(runs);
            if index < state.count {
                state.lines.insert(index, line.clone());
            }
            return line;
        }
        if index > state.count {
            return TerminalLine::default();
        }
        state
            .lines
            .get(&index)
            .cloned()
            .unwrap_or_else(|| state.error_line())
    }
    fn cursor(&self) -> Option<TerminalCursor> {
        self.0
            .borrow_mut()
            .query("cursor", |kernel| kernel.cursor())
            .flatten()
            .map(|(col, row)| {
                let mut cursor = TerminalCursor::default();
                cursor.col = col;
                cursor.row = row;
                cursor
            })
    }
    fn modes(&self) -> TerminalModes {
        let mut state = self.0.borrow_mut();
        if let Some((application_cursor, bracketed_paste)) =
            state.query("input modes", |kernel| kernel.modes())
        {
            state.modes.application_cursor = application_cursor;
            state.modes.bracketed_paste = bracketed_paste;
        }
        state.modes
    }
    fn take_events(&mut self) -> Vec<TerminalEmulatorEvent> {
        let state = self.0.get_mut();
        let mut events = state
            .kernel
            .as_mut()
            .map(Emulator::take_events)
            .unwrap_or_default()
            .into_iter()
            .map(|event| match event {
                Event::Title(title) => TerminalEmulatorEvent::Title(title),
                Event::Bell => TerminalEmulatorEvent::Bell,
            })
            .collect::<Vec<_>>();
        if std::mem::take(&mut state.failure_pending)
            && let Some(failure) = &state.failure
        {
            events.push(TerminalEmulatorEvent::Title(failure.to_string()));
            events.push(TerminalEmulatorEvent::Bell);
        }
        events
    }
}

pub fn initialize() -> Result<TerminalEmulatorFactory, Error> {
    let mut kernel = Emulator::new(80, 24, 1000)?;
    kernel.line(0)?;
    kernel.cursor()?;
    kernel.modes()?;
    Ok(factory())
}

pub fn factory() -> TerminalEmulatorFactory {
    TerminalEmulatorFactory::new(|size, scrollback| {
        Box::new(
            GhosttyEmulator::new(size, scrollback)
                .unwrap_or_else(|error| GhosttyEmulator::failed(size, error)),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_never_replies_to_queries_including_replay() {
        let mut emulator = GhosttyEmulator::new(TerminalSize::new(80, 24), 100).unwrap();
        for _ in 0..2 {
            emulator.feed(b"\x1b[5n\x1b[6n\x1b[c\x1b]0;title\x07\x07");
            let events = emulator.take_events();
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, TerminalEmulatorEvent::Reply(_)))
            );
            assert!(events.iter().any(
                |event| matches!(event, TerminalEmulatorEvent::Title(title) if title == "title")
            ));
            assert!(events.contains(&TerminalEmulatorEvent::Bell));
        }
    }

    #[test]
    fn trait_maps_wide_text_styles_modes_and_cursor() {
        let mut emulator = GhosttyEmulator::new(TerminalSize::new(80, 24), 100).unwrap();
        emulator.feed("\x1b[38;2;12;34;56m中a\x1b[?1h\x1b[?2004h".as_bytes());
        assert!(emulator.line(0).text().starts_with("中a"));
        assert_eq!(
            emulator.line(0).runs[0].style.fg,
            Some(TerminalColor::Rgb(12, 34, 56))
        );
        assert_eq!(emulator.cursor().unwrap().col, 3);
        assert!(emulator.modes().application_cursor && emulator.modes().bracketed_paste);
        emulator.feed(b"\r\x1b[0;31;48;5;200mX");
        let style = emulator.line(0).runs[0].style;
        assert_eq!(style.fg, Some(TerminalColor::Indexed(1)));
        assert_eq!(style.bg, Some(TerminalColor::Indexed(200)));
    }

    #[test]
    fn failure_freezes_rendered_content_and_reports_once() {
        let mut emulator = GhosttyEmulator::new(TerminalSize::new(80, 24), 100).unwrap();
        emulator.feed(b"keep this output\x1b[?2004h");
        let count = emulator.line_count();
        let saved = emulator.line(0);
        assert!(emulator.modes().bracketed_paste);
        emulator.0.get_mut().fail("render line", Error::OutOfMemory);
        emulator.feed(b"must not replace saved output");
        emulator.resize(TerminalSize::new(40, 10));
        assert_eq!(emulator.line(0), saved);
        assert_eq!(emulator.line_count(), count + 1);
        assert!(emulator.line(count).text().contains("out of memory"));
        assert!(emulator.cursor().is_none());
        assert!(emulator.modes().bracketed_paste);
        assert_eq!(emulator.take_events().len(), 2);
        assert!(emulator.take_events().is_empty());
        assert_eq!(emulator.failure().unwrap().operation, "render line");
    }

    #[test]
    fn invalid_resize_and_initialization_are_controlled() {
        let mut size = TerminalSize::default();
        size.cols = 0;
        assert!(GhosttyEmulator::new(size, 100).is_err());
        let mut emulator = GhosttyEmulator::new(TerminalSize::default(), 100).unwrap();
        emulator.resize(size);
        assert_eq!(emulator.failure().unwrap().operation, "resize");
        assert!(initialize().is_ok());
        let failed = GhosttyEmulator::failed(TerminalSize::default(), Error::OutOfMemory);
        assert!(
            failed
                .line(failed.line_count() - 1)
                .text()
                .contains("initialize")
        );
    }
}
