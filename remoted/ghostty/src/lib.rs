//! Native terminal snapshots without a PTY reply callback.

pub mod kitty;

pub use kitty::initialize_graphics;

use libghostty_vt::{
    Terminal,
    mode::Mode,
    render::{CellIterator, Overscan, RenderState, RowIterator},
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

/// A renderable native cell. Wide-character tails are omitted; blank slots
/// (including wide-wrap head spacers) retain their native column and style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Zero-based column in the native row, including omitted tail slots.
    pub column: usize,
    /// Occupied native columns, independent of Unicode or font measurements.
    pub width: usize,
    /// The complete native grapheme, or one space for a blank slot.
    pub text: String,
    /// Resolved native style, with unchanged palette indices where possible.
    pub style: Style,
}

struct Row {
    runs: Vec<Run>,
    cells: Vec<Cell>,
}

pub struct Emulator {
    terminal: Terminal<'static, 'static>,
    render: RenderState<'static>,
    cache: (usize, Vec<Row>),
    events: Rc<RefCell<Vec<Event>>>,
    images: kitty::Images,
    cell_pixels: (u32, u32),
}

impl Emulator {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Result<Self, Error> {
        initialize_graphics()?;
        if cols == 0 || rows == 0 {
            return Err(Error::InvalidValue);
        }
        let mut terminal = Terminal::new(cols, rows)?;
        kitty::configure(&mut terminal)?;
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
            images: kitty::Images::new()?,
            cell_pixels: (0, 0),
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
        self.terminal
            .resize(cols, rows, self.cell_pixels.0, self.cell_pixels.1)?;
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
        Ok(self
            .row(index)?
            .map(|row| row.runs.clone())
            .unwrap_or_default())
    }

    /// Read native cells without changing the cursor, modes, or reply authority.
    ///
    /// Returns an empty vector for a row outside the screen and scrollback.
    ///
    /// # Errors
    /// Returns the native error if the render snapshot cannot be read.
    pub fn cells(&mut self, index: usize) -> Result<Vec<Cell>, Error> {
        Ok(self
            .row(index)?
            .map(|row| row.cells.clone())
            .unwrap_or_default())
    }

    /// Read Kitty placements in global scrollback pixels without emitting VT replies.
    ///
    /// `visible_rows` uses global scrollback rows; pixel metrics must be positive.
    /// Native overscan includes the full range when the client is taller than the PTY.
    /// The viewport and overscan request are restored after reading the snapshot.
    ///
    /// # Errors
    /// Returns a native snapshot error or invalid geometry. Image data stays bounded.
    pub fn images(
        &mut self,
        visible_rows: std::ops::Range<usize>,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<Vec<kitty::Placement>, Error> {
        if cell_width == 0 || cell_height == 0 || cell_width > 4096 || cell_height > 4096 {
            return Err(Error::InvalidValue);
        }
        if self.cell_pixels != (cell_width, cell_height) {
            let synchronized = self.terminal.mode(Mode::SYNC_OUTPUT)?;
            self.terminal.resize(
                self.terminal.cols()?,
                self.terminal.rows()?,
                cell_width,
                cell_height,
            )?;
            if synchronized {
                self.terminal.set_mode(Mode::SYNC_OUTPUT, true)?;
            }
            self.cell_pixels = (cell_width, cell_height);
        }
        let end = visible_rows.end.min(self.line_count()?);
        if visible_rows.start >= end {
            return Ok(Vec::new());
        }
        let first_row = visible_rows.start.min(self.terminal.scrollback_rows()?);
        let below = end.saturating_sub(first_row + usize::from(self.terminal.rows()?));
        self.render.set_overscan(Overscan {
            above: 0,
            below: u16::try_from(below).map_err(|_| Error::InvalidValue)?,
        })?;
        self.terminal
            .scroll_viewport(ScrollViewport::Row(first_row));
        // Native updates consume terminal dirty flags. Text and images must
        // share this one retained state so either read preserves the other.
        let result = (|| {
            let snapshot = self.render.update(&self.terminal)?;
            self.images.snapshot(
                &self.terminal,
                &snapshot,
                first_row,
                cell_width,
                cell_height,
            )
        })();
        self.terminal.scroll_viewport(ScrollViewport::Bottom);
        // Row text caches expect exactly one source viewport, including after
        // an image iterator error. Changing overscan forces its next full update.
        let reset = self
            .render
            .set_overscan(Overscan { above: 0, below: 0 })
            .map(|_| ());
        result.and_then(|images| reset.map(|()| images))
    }

    fn row(&mut self, index: usize) -> Result<Option<&Row>, Error> {
        if index >= self.line_count()? {
            return Ok(None);
        }
        if index < self.cache.0 || index - self.cache.0 >= self.cache.1.len() {
            let start = index.min(self.terminal.scrollback_rows()?);
            self.terminal.scroll_viewport(ScrollViewport::Row(start));
            let result = self.read_viewport();
            self.terminal.scroll_viewport(ScrollViewport::Bottom);
            self.cache = (start, result?);
        }
        Ok(self.cache.1.get(index - self.cache.0))
    }

    fn read_viewport(&mut self) -> Result<Vec<Row>, Error> {
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
            let mut native_cells = Vec::new();
            let mut column = 0;
            while let Some(cell) = cells.next() {
                // The iterator visits every native slot, including wide tails.
                // Never infer columns from the text of a cell or a whole run.
                let native_column = column;
                column += 1;
                let raw = cell.raw_cell()?;
                let width = match raw.wide()? {
                    CellWide::Wide => 2,
                    CellWide::Narrow | CellWide::SpacerHead => 1,
                    CellWide::SpacerTail => continue,
                };
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
                    runs.push(Run {
                        text: text.clone(),
                        style,
                    });
                }
                native_cells.push(Cell {
                    column: native_column,
                    width,
                    text,
                    style,
                });
            }
            lines.push(Row {
                runs,
                cells: native_cells,
            });
        }
        Ok(lines)
    }
}
