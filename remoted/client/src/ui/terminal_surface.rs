use nsh_base::{
    Terminal, TerminalApi, TerminalCursor, TerminalEmulator, TerminalEmulatorEvent,
    TerminalEmulatorFactory, TerminalLine, TerminalModes, TerminalSize,
};
use nsh_core::prelude::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub struct Buffer {
    emulator: Rc<RefCell<crate::terminal::GhosttyEmulator>>,
    pub revision: Signal<u64>,
    pub size: Signal<TerminalSize>,
}

impl Buffer {
    fn new() -> Self {
        Self {
            emulator: Rc::new(RefCell::new(
                crate::terminal::GhosttyEmulator::new(TerminalSize::default(), 1000)
                    .unwrap_or_else(|error| {
                        crate::terminal::GhosttyEmulator::failed(TerminalSize::default(), error)
                    }),
            )),
            revision: signal(0),
            size: signal(TerminalSize::default()),
        }
    }
    fn feed(&self, data: &[u8]) {
        self.emulator.borrow_mut().feed(data);
        self.revision
            .update(|revision| *revision = revision.wrapping_add(1));
    }
    pub fn resize(&self, size: TerminalSize) {
        if self.size.get_untracked() != size {
            self.emulator.borrow_mut().resize(size);
            if self.emulator.borrow().failure().is_none() {
                self.size.set(size);
            }
            self.revision
                .update(|revision| *revision = revision.wrapping_add(1));
        }
    }
    pub fn line_count(&self) -> usize {
        self.revision.get();
        self.emulator.borrow().line_count()
    }
    pub fn line(&self, index: usize) -> TerminalLine {
        self.revision.get();
        self.emulator.borrow().line(index)
    }
    pub fn cursor(&self) -> Option<TerminalCursor> {
        self.revision.get();
        self.emulator.borrow().cursor()
    }
    #[cfg(test)]
    pub fn modes(&self) -> TerminalModes {
        self.emulator.borrow().modes()
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

struct SharedEmulator {
    buffer: Buffer,
    viewport: TerminalSize,
}
impl TerminalEmulator for SharedEmulator {
    fn feed(&mut self, data: &[u8]) {
        self.buffer.feed(data);
    }
    fn resize(&mut self, size: TerminalSize) {
        // NSH clamps a not-yet-laid-out or hidden terminal's zero bounds to
        // 1x1. Applying that placeholder would irreversibly truncate the shared
        // VT before the visible viewport is measured.
        if size.cols == 1 && size.rows == 1 {
            return;
        }
        self.viewport = size;
        self.buffer.resize(size);
    }
    fn line_count(&self) -> usize {
        // NSH invalidates its rows after releasing its emulator borrow. Tracking
        // the shared buffer here would also invalidate them inside feed/resize.
        untracked(|| self.buffer.line_count()).max(self.viewport.rows as usize)
    }
    fn line(&self, index: usize) -> TerminalLine {
        let line = untracked(|| self.buffer.line(index));
        // Theme reads must stay tracked so existing rows redraw without new PTY output.
        crate::preferences::current_context().terminal_line(line)
    }
    fn cursor(&self) -> Option<TerminalCursor> {
        (self.viewport == self.buffer.size.get_untracked())
            .then(|| untracked(|| self.buffer.cursor()))
            .flatten()
    }
    fn modes(&self) -> TerminalModes {
        self.buffer.emulator.borrow().modes()
    }
    fn take_events(&mut self) -> Vec<TerminalEmulatorEvent> {
        self.buffer.events()
    }
}

#[derive(Clone)]
pub struct Surface {
    pub reference: ComponentRef<Terminal>,
    pub buffer: Buffer,
}
impl Surface {
    pub fn new() -> Self {
        Self {
            reference: component_ref::<Terminal>(),
            buffer: Buffer::new(),
        }
    }
    pub fn factory(&self) -> TerminalEmulatorFactory {
        let buffer = self.buffer.clone();
        TerminalEmulatorFactory::new(move |size, _| {
            Box::new(SharedEmulator {
                buffer: buffer.clone(),
                viewport: size,
            })
        })
    }
    pub fn write(&self, text: String) {
        if let Some(api) = self.reference.api::<TerminalApi>() {
            api.write(text);
        } else {
            self.buffer.feed(text.as_bytes());
            self.buffer.events();
        }
    }
    pub fn reset(&self) {
        self.write("\x1bc".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::element::Root;
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;

    #[component]
    fn PlaceholderResizeFixture() -> impl IntoElement {
        let buffer = Buffer::new();
        let mut emulator = SharedEmulator {
            buffer: buffer.clone(),
            viewport: TerminalSize::default(),
        };
        emulator.feed(b"developer@neko:~/project$ git status\r\nWorking tree clean");
        let original = buffer.text();
        emulator.resize(TerminalSize::new(1, 1));
        let placeholder_preserved =
            buffer.text() == original && buffer.size.get_untracked() == TerminalSize::default();
        emulator.resize(TerminalSize::new(105, 27));
        let visible_preserved = buffer
            .text()
            .contains("developer@neko:~/project$ git status")
            && buffer.text().contains("Working tree clean")
            && buffer.size.get_untracked() == TerminalSize::new(105, 27);
        text(format!(
            "placeholder={placeholder_preserved};visible={visible_preserved}"
        ))
    }

    #[test]
    fn placeholder_resize_preserves_shared_terminal_output() {
        let mut app = Headless::start(EngineConfig::new(Root::of::<PlaceholderResizeFixture>()))
            .expect("start resize regression fixture");
        app.first_render()
            .expect("render resize regression fixture");
        assert!(app.shows("placeholder=true;visible=true"));
    }

    #[component]
    fn SharedBufferFixture() -> impl IntoElement {
        let buffer = Buffer::new();
        let mut first = SharedEmulator {
            buffer: buffer.clone(),
            viewport: TerminalSize::default(),
        };
        let mut second = SharedEmulator {
            buffer: buffer.clone(),
            viewport: TerminalSize::default(),
        };
        first.feed("共享 buffer".as_bytes());
        let shared = second.line(0).text().starts_with("共享 buffer");
        first.resize(TerminalSize::new(100, 30));
        let stale_cursor_hidden = second.cursor().is_none() && first.cursor().is_some();
        second.resize(TerminalSize::new(1, 1));
        let placeholder_preserved = buffer.size.get_untracked() == TerminalSize::new(100, 30);
        second.resize(TerminalSize::new(100, 30));
        text(format!(
            "shared={shared};stale={stale_cursor_hidden};placeholder={placeholder_preserved};cursor={}",
            second.cursor().is_some()
        ))
    }

    #[test]
    fn views_share_output_and_only_show_the_current_viewport_cursor() {
        let mut app = Headless::start(EngineConfig::new(Root::of::<SharedBufferFixture>()))
            .expect("start shared buffer fixture");
        app.first_render().expect("render shared buffer fixture");
        assert!(app.shows("shared=true;stale=true;placeholder=true;cursor=true"));
    }
}
