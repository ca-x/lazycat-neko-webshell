#[cfg(test)]
use nsh_base::TerminalLine;
use nsh_base::{
    TerminalCursor, TerminalEmulator, TerminalEmulatorEvent, TerminalModes, TerminalSize,
};
use nsh_core::prelude::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub struct Buffer {
    emulator: Rc<RefCell<crate::terminal::GhosttyEmulator>>,
    pub revision: Signal<u64>,
    pub size: Signal<TerminalSize>,
    pub clipped: Signal<bool>,
    failure: Signal<Option<crate::terminal::Failure>>,
}

impl Buffer {
    fn new() -> Self {
        let emulator = crate::terminal::GhosttyEmulator::new(TerminalSize::default(), 1000)
            .unwrap_or_else(|error| {
                crate::terminal::GhosttyEmulator::failed(TerminalSize::default(), error)
            });
        let failure = signal(emulator.failure());
        Self {
            emulator: Rc::new(RefCell::new(emulator)),
            revision: signal(0),
            clipped: signal(false),
            size: signal(TerminalSize::default()),
            failure,
        }
    }

    fn publish_failure(&self, failure: Option<crate::terminal::Failure>) {
        if failure.is_some() && self.failure.get_untracked().is_none() {
            self.failure.set(failure);
        }
    }

    fn read<Value>(&self, read: impl FnOnce(&crate::terminal::GhosttyEmulator) -> Value) -> Value {
        self.revision.get();
        let (value, failure) = {
            let emulator = self.emulator.borrow();
            (read(&emulator), emulator.failure())
        };
        // Subscribers may immediately read or resize the VT. Release its borrow
        // before publishing a first failure, independently of future PTY output.
        self.publish_failure(failure);
        value
    }

    fn feed(&self, data: &[u8]) {
        let failure = {
            let mut emulator = self.emulator.borrow_mut();
            emulator.feed(data);
            emulator.failure()
        };
        self.publish_failure(failure);
        self.revision
            .update(|revision| *revision = revision.wrapping_add(1));
    }
    pub fn resize(&self, size: TerminalSize) {
        if self.size.get_untracked() != size {
            let failure = {
                let mut emulator = self.emulator.borrow_mut();
                emulator.resize(size);
                emulator.failure()
            };
            let resized = failure.is_none();
            self.publish_failure(failure);
            if resized {
                self.size.set(size);
            }
            self.revision
                .update(|revision| *revision = revision.wrapping_add(1));
        }
    }
    pub fn line_count(&self) -> usize {
        self.read(|emulator| emulator.line_count())
    }
    #[cfg(test)]
    pub fn line(&self, index: usize) -> TerminalLine {
        self.read(|emulator| emulator.line(index))
    }
    pub fn cells(&self, index: usize) -> Vec<crate::terminal::GridCell> {
        self.read(|emulator| emulator.cells(index))
    }
    pub fn images(
        &self,
        visible_rows: std::ops::Range<usize>,
        cell_width: u32,
        cell_height: u32,
    ) -> Vec<neko_ghostty::kitty::Placement> {
        self.read(|emulator| emulator.images(visible_rows, cell_width, cell_height))
    }
    pub fn failure(&self) -> Option<crate::terminal::Failure> {
        self.failure.get()
    }
    pub fn cursor(&self) -> Option<TerminalCursor> {
        self.read(|emulator| emulator.cursor())
    }
    pub fn modes(&self) -> TerminalModes {
        self.read(|emulator| emulator.modes())
    }
    fn events(&self) -> Vec<TerminalEmulatorEvent> {
        self.emulator
            .borrow_mut()
            .take_events()
            .into_iter()
            .filter(|event| !matches!(event, TerminalEmulatorEvent::Reply(_)))
            .collect()
    }
    #[cfg(test)]
    pub fn text(&self) -> String {
        (0..self.line_count())
            .map(|index| self.line(index).text().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone)]
pub struct Surface {
    pub buffer: Buffer,
    screen: Option<Rc<RefCell<super::screen_text::ScreenText>>>,
}
impl Surface {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::for_screen(false)
    }
    pub fn for_screen(snapshot: bool) -> Self {
        Self {
            buffer: Buffer::new(),
            screen: snapshot
                .then(|| Rc::new(RefCell::new(super::screen_text::ScreenText::default()))),
        }
    }
    pub fn write(&self, text: String) {
        let text = if let Some(screen) = &self.screen {
            let mut screen = screen.borrow_mut();
            let (text, size) = screen.push(&text);
            self.buffer.clipped.set(screen.clipped);
            self.buffer.resize(size);
            text
        } else {
            text
        };
        self.buffer.feed(text.as_bytes());
        self.buffer.events();
    }
    pub fn reset(&self) {
        self.buffer.clipped.set(false);
        if let Some(screen) = &self.screen {
            *screen.borrow_mut() = super::screen_text::ScreenText::default();
        }
        // Reset is terminal control, never part of the snapshot's column count.
        self.buffer.feed(b"\x1bc");
        if self.screen.is_some() {
            // Even an empty replacement must forget the previous source geometry.
            self.buffer.resize(TerminalSize::default());
        }
        self.buffer.events();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::element::Root;
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;

    #[test]
    fn ansi_replacement_preserves_real_colors_and_reset_forgets_styles_and_geometry() {
        use nsh_base::TerminalColor;
        let runtime = nsh_core::element::reactive::Runtime::default();
        let _entered = runtime.enter();
        let surface = Surface::for_screen(true);
        surface.write("\x1b]0;title with a long payload\x1b".into());
        surface.write("\\\x1b[38;2;12;34;".into());
        surface.write(format!("56m\x1b[48;5;4m{}\x1b[0m\nlast", "猫".repeat(60)));
        assert_eq!(surface.buffer.size.get_untracked().cols, 120);
        assert!(!surface.buffer.clipped.get_untracked());
        assert_eq!(surface.buffer.line(0).text().trim_end(), "猫".repeat(60));
        assert_eq!(surface.buffer.line(1).text().trim_end(), "last");
        let first = surface.buffer.cells(0).remove(0);
        assert_eq!(first.style.fg, Some(TerminalColor::Rgb(12, 34, 56)));
        assert_eq!(first.style.bg, Some(TerminalColor::Indexed(4)));
        surface.reset();
        assert_eq!(surface.buffer.size.get_untracked(), TerminalSize::default());
        assert!(surface.buffer.text().trim().is_empty());
        surface.write("plain replacement".into());
        let first = surface.buffer.cells(0).remove(0);
        assert_ne!(first.style.fg, Some(TerminalColor::Rgb(12, 34, 56)));
        assert_ne!(first.style.bg, Some(TerminalColor::Indexed(4)));
        assert!(!surface.buffer.text().contains("猫"));
        surface.write("\x1b[38;2;".into());
        surface.reset();
        surface.write("after incomplete escape".into());
        assert_eq!(
            surface.buffer.line(0).text().trim_end(),
            "after incomplete escape"
        );
    }

    #[component]
    fn SnapshotFixture() -> impl IntoElement {
        let screen = Surface::for_screen(true);
        screen.write("one\ntwo\r".into());
        screen.write("\n猫\n".into());
        let aligned = screen.buffer.line(0).text().trim_end() == "one"
            && screen.buffer.line(1).text().trim_end() == "two"
            && screen.buffer.line(2).text().trim_end() == "猫";
        screen.reset();
        screen.write(format!("{}\n{}", "猫".repeat(70), "line\n".repeat(50)));
        let source = screen.buffer.size.get_untracked();
        let wide = screen.buffer.line(0).text().trim_end() == "猫".repeat(70)
            && screen.buffer.line(1).text().trim_end() == "line";
        screen.reset();
        screen.write("replacement\nlast".into());
        let replaced = screen.buffer.line(0).text().trim_end() == "replacement"
            && screen.buffer.line(1).text().trim_end() == "last"
            && !screen.buffer.text().contains("猫");
        let pty = Surface::new();
        pty.write("one\ntwo".into());
        let raw = pty.buffer.line(1).text().starts_with("   two");
        text(format!(
            "aligned={aligned};wide={wide};source={}x{};replaced={replaced};raw={raw}",
            source.cols, source.rows
        ))
    }

    #[test]
    fn herdr_snapshots_align_and_replace_without_changing_raw_pty_line_feeds() {
        let mut app = Headless::start(EngineConfig::new(Root::of::<SnapshotFixture>()))
            .expect("start snapshot fixture");
        app.first_render().expect("render snapshot fixture");
        assert!(app.shows("aligned=true;wide=true;source=140x52;replaced=true;raw=true"));
    }

    #[component]
    fn ReadFailureFixture() -> impl IntoElement {
        let surface = Surface::new();
        surface.write("中e\u{301} keep".into());
        let buffer = surface.buffer.clone();
        let saved = buffer.cells(0);
        let notice_buffer = buffer.clone();
        let expected = saved.clone();
        let notices = signal(0usize);
        effect(move || {
            if notice_buffer.failure().is_some() {
                // Publishing a failure must allow subscribers to borrow the VT again.
                assert!(notice_buffer.emulator.try_borrow_mut().is_ok());
                assert_eq!(notice_buffer.cells(0), expected);
                notices.update(|count| *count += 1);
            }
        });
        let status = signal(String::new());
        view()
            .flex_1()
            .child(text("inject-read-failure").on_press(move |_: &PressEvent| {
                let revision = buffer.revision.get_untracked();
                buffer.emulator.borrow().fail_next_query("render cells");
                let retained = buffer.cells(0) == saved;
                status.set(format!(
                    "retained={retained};no_output={}",
                    buffer.revision.get_untracked() == revision
                ));
            }))
            .child(text(move || {
                format!("{};notices={}", status.get(), notices.get())
            }))
            .child(super::super::terminal_observer::render(
                surface.buffer,
                signal(None),
                || false,
            ))
    }

    #[test]
    fn read_failure_notifies_once_without_new_output_and_keeps_cells() {
        let mut app = Headless::start(EngineConfig::new(Root::of::<ReadFailureFixture>()))
            .expect("start read failure fixture");
        app.first_render().expect("render read failure fixture");
        assert!(!app.shows("Ghostty stopped"));
        for _ in 0..2 {
            app.press("inject-read-failure")
                .expect("inject a read failure");
            assert!(app.shows("Ghostty stopped (render cells): out of memory"));
            assert!(app.shows("retained=true;no_output=true;notices=1"));
        }
    }

    #[test]
    fn every_read_query_publishes_failure_without_changing_revision() {
        let runtime = nsh_core::element::reactive::Runtime::default();
        let _entered = runtime.enter();
        for operation in [
            "line count",
            "render line",
            "render cells",
            "cursor",
            "input modes",
        ] {
            let buffer = Buffer::new();
            buffer.feed("中e\u{301} keep".as_bytes());
            let saved = buffer.cells(0);
            let failures = buffer.clone();
            let failure = computed(move || failures.failure());
            assert!(failure.get().is_none());
            let revision = buffer.revision.get_untracked();
            buffer.emulator.borrow().fail_next_query(operation);
            match operation {
                "line count" => {
                    buffer.line_count();
                }
                "render line" => {
                    buffer.line(0);
                }
                "render cells" => {
                    buffer.cells(0);
                }
                "cursor" => {
                    buffer.cursor();
                }
                "input modes" => {
                    buffer.modes();
                }
                _ => unreachable!(),
            }
            assert_eq!(failure.get().unwrap().operation, operation);
            assert_eq!(buffer.revision.get_untracked(), revision);
            assert_eq!(buffer.cells(0), saved);
        }
    }
}
