use libghostty_vt::{
    Terminal,
    mode::Mode,
    render::{CellIterator, RenderState, RowIterator},
    screen::{CellContentTag, CellWide},
    terminal::ScrollViewport,
};
use std::{cell::RefCell, rc::Rc};

pub use libghostty_vt::{
    error::Error,
    style::{Style, StyleColor, Underline},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Title(String),
    Bell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

pub struct Emulator {
    terminal: Terminal<'static, 'static>,
    render: RenderState<'static>,
    cache: (usize, Vec<Vec<Run>>),
    events: Rc<RefCell<Vec<Event>>>,
}

impl Emulator {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Result<Self, Error> {
        if cols == 0 || rows == 0 {
            return Err(Error::InvalidValue);
        }
        let mut terminal = Terminal::new(cols, rows)?;
        terminal.set_scrollback_max_bytes(Some(usize::MAX))?;
        terminal.set_scrollback_max_lines(Some(scrollback))?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let bells = events.clone();
        terminal.on_bell(move |_| bells.borrow_mut().push(Event::Bell))?;
        let titles = events.clone();
        terminal.on_title_changed(move |terminal: &Terminal<'_, '_>| {
            if let Ok(title) = terminal.title() {
                titles
                    .borrow_mut()
                    .push(Event::Title(String::from_utf8_lossy(title).into_owned()));
            }
        })?;
        Ok(Self {
            terminal,
            render: RenderState::new()?,
            cache: (0, Vec::new()),
            events,
        })
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.vt_write(bytes);
        self.cache.1.clear();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        if cols == 0 || rows == 0 {
            return Err(Error::InvalidValue);
        }
        self.terminal.resize(cols, rows, 0, 0)?;
        self.cache.1.clear();
        Ok(())
    }

    pub fn line_count(&self) -> Result<usize, Error> {
        self.terminal.total_rows()
    }

    pub fn processing_error(&self) -> Result<bool, Error> {
        self.terminal.vt_processing_error()
    }

    pub fn cursor(&self) -> Result<Option<(u16, u16)>, Error> {
        self.terminal
            .is_cursor_visible()?
            .then(|| Ok((self.terminal.cursor_x()?, self.terminal.cursor_y()?)))
            .transpose()
    }

    pub fn modes(&self) -> Result<(bool, bool), Error> {
        Ok((
            self.terminal.mode(Mode::DECCKM)?,
            self.terminal.mode(Mode::BRACKETED_PASTE)?,
        ))
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    pub fn line(&mut self, index: usize) -> Result<Vec<Run>, Error> {
        if index >= self.line_count()? {
            return Ok(Vec::new());
        }
        let (start, lines) = &self.cache;
        if let Some(line) = index
            .checked_sub(*start)
            .and_then(|offset| lines.get(offset))
        {
            return Ok(line.clone());
        }
        let start = index.min(self.terminal.scrollback_rows()?);
        self.terminal.scroll_viewport(ScrollViewport::Row(start));
        let result = self.read_viewport();
        self.terminal.scroll_viewport(ScrollViewport::Bottom);
        let lines = result?;
        let line = lines.get(index - start).cloned().unwrap_or_default();
        self.cache = (start, lines);
        Ok(line)
    }

    fn read_viewport(&mut self) -> Result<Vec<Vec<Run>>, Error> {
        let palette = self.terminal.color_palette()?;
        let defaults = self.terminal.default_color_palette()?;
        let resolve = |color: StyleColor, default: Option<libghostty_vt::color::Rgb>| match color {
            StyleColor::Palette(index) if palette.get(index) != defaults.get(index) => {
                StyleColor::Rgb(palette.get(index))
            }
            StyleColor::None => default.map_or(StyleColor::None, StyleColor::Rgb),
            color => color,
        };
        let foreground = self.terminal.fg_color()?;
        let background = self.terminal.bg_color()?;
        let snapshot = self.render.update(&self.terminal)?;
        let mut rows = RowIterator::new()?;
        let mut rows = rows.update(&snapshot)?;
        let mut cells = CellIterator::new()?;
        let mut lines = Vec::new();
        while let Some(row) = rows.next() {
            let mut cells = cells.update(row)?;
            let mut runs: Vec<Run> = Vec::new();
            while let Some(cell) = cells.next() {
                let raw = cell.raw_cell()?;
                let wide = raw.wide()?;
                if wide == CellWide::SpacerTail {
                    continue;
                }
                let mut style = cell.style()?;
                style.fg_color = resolve(style.fg_color, foreground);
                style.bg_color = resolve(
                    match raw.content_tag()? {
                        CellContentTag::BgColorPalette => {
                            StyleColor::Palette(raw.bg_color_palette()?)
                        }
                        CellContentTag::BgColorRgb => StyleColor::Rgb(raw.bg_color_rgb()?),
                        _ => style.bg_color,
                    },
                    background,
                );
                let mut text = String::new();
                cell.graphemes_utf8(&mut text)?;
                if text.is_empty() {
                    text.push(' ');
                }
                if let Some(run) = runs.last_mut().filter(|run| run.style == style) {
                    run.text.push_str(&text);
                } else {
                    runs.push(Run { text, style });
                }
            }
            lines.push(runs);
        }
        Ok(lines)
    }
}
