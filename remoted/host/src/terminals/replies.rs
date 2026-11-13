//! Thread-local VT reply authority. Feed live PTY bytes once, never replay.
//!
//! The renderer has no reply callback. OSC 7501 remains owned by program-status;
//! Ghostty handles standard VT queries here without a second handwritten parser.
use allocator_api2::alloc::{AllocError, Allocator, Global, Layout};
use anyhow::{Context, Result, bail, ensure};
use libghostty_vt::{Terminal, alloc::Allocator as GhosttyAllocator, report::SizeReportSize};
use std::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    rc::Rc,
};

const MEMORY_BYTES: usize = 16 * 1024 * 1024;
const SEQUENCE_BYTES: usize = 16 * 1024;
const REPLY_BYTES: usize = 16 * 1024;
const INPUT_BYTES: usize = 16 * 1024;
const MAX_DIMENSION: u16 = 1000;
const MAX_CELLS: usize = 128 * 1024;

/// Owns the allocation budget on the I/O thread's stack. The scoped entry point
/// keeps both the allocator and callback alive until the terminal is destroyed.
struct Budget {
    limit: usize,
    used: Cell<usize>,
    exhausted: Cell<bool>,
}

// SAFETY: Every allocation/deallocation uses Global with the same layout. The
// budget only rejects allocation; it never moves or aliases a live allocation.
// Cell intentionally keeps this allocator confined to its owning thread.
unsafe impl Allocator for Budget {
    fn allocate(&self, layout: Layout) -> std::result::Result<NonNull<[u8]>, AllocError> {
        let size = layout.size().max(1);
        let Some(used) = self
            .used
            .get()
            .checked_add(size)
            .filter(|n| *n <= self.limit)
        else {
            self.exhausted.set(true);
            return Err(AllocError);
        };
        let allocation = Global
            .allocate(layout)
            .inspect_err(|_| self.exhausted.set(true))?;
        self.used.set(used);
        Ok(allocation)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: Forward the exact pointer/layout pair supplied by Ghostty's
        // allocator adapter to the allocator that originally allocated it.
        unsafe { Global.deallocate(ptr, layout) };
        self.used.set(self.used.get() - layout.size().max(1));
    }
}

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    overflow: bool,
    callback_error: bool,
}

pub(super) struct Replies<'a> {
    terminal: Terminal<'a, 'a>,
    filter: neko_terminal_reply_filter::ReplyOnlyInputFilter,
    output: Rc<RefCell<Output>>,
    budget: &'a Budget,
    failed: bool,
}

/// Create on the I/O thread, then run its live event loop inside `run`.
/// `observe`/`resize` return at most 16 KiB for the existing bounded writer queue.
/// A queue/write error must fail the PTY authority, never retry a parsed chunk.
pub(super) fn with<R>(
    cols: u16,
    rows: u16,
    run: impl FnOnce(&mut Replies<'_>) -> Result<R>,
) -> Result<R> {
    with_budget(cols, rows, MEMORY_BYTES, run)
}

pub(super) fn dimensions(cols: u16, rows: u16) -> Result<()> {
    ensure!(
        cols > 0
            && rows > 0
            && cols <= MAX_DIMENSION
            && rows <= MAX_DIMENSION
            && usize::from(cols) * usize::from(rows) <= MAX_CELLS,
        "PTY VT reply dimensions exceed bounded screen capacity"
    );
    Ok(())
}

fn with_budget<R>(
    cols: u16,
    rows: u16,
    limit: usize,
    run: impl FnOnce(&mut Replies<'_>) -> Result<R>,
) -> Result<R> {
    dimensions(cols, rows)?;
    neko_ghostty::initialize_graphics().context("cannot initialize bounded Kitty PNG decoder")?;
    let budget = Budget {
        limit,
        used: Cell::new(0),
        exhausted: Cell::new(false),
    };
    let allocator = GhosttyAllocator::from(&budget);
    let mut terminal = Terminal::new_with_alloc(&allocator, cols, rows)
        .context("cannot create bounded PTY VT reply authority")?;
    terminal.set_scrollback_max_bytes(Some(0))?;
    terminal.set_scrollback_max_lines(Some(0))?;
    terminal.set_continuation_max_bytes(SEQUENCE_BYTES)?;
    terminal.set_apc_max_bytes(Some(SEQUENCE_BYTES))?;
    terminal.set_apc_max_bytes_kitty(Some(neko_terminal_reply_filter::MAX_KITTY_QUERY_BYTES))?;
    // A nonzero limit keeps support queries enabled. The query-only filter
    // never admits image transmissions, so no images are retained here.
    terminal.set_kitty_image_storage_limit(1)?;
    terminal.set_kitty_image_from_file_allowed(false)?;
    terminal.set_kitty_image_temp_file_dir(None)?;
    terminal.set_kitty_image_from_shared_mem_allowed(false)?;
    terminal.set_unknown_max_bytes(0)?;
    terminal.set_clipboard_write_max_bytes(Some(0))?;
    terminal.set_glyph_protocol_enabled(false)?;
    terminal.set_terminfo_name("xterm-256color")?;
    let output = Rc::new(RefCell::new(Output::default()));
    let sizes = output.clone();
    terminal.on_size(move |term| match (term.cols(), term.rows()) {
        (Ok(columns), Ok(rows)) => Some(SizeReportSize {
            rows,
            columns,
            // Native PTYs only supply cell counts; do not invent pixel sizes.
            cell_width: 0,
            cell_height: 0,
        }),
        _ => {
            sizes.borrow_mut().callback_error = true;
            None
        }
    })?;
    let writes = output.clone();
    terminal.on_pty_write(move |_, bytes| {
        let mut output = writes.borrow_mut();
        if output.overflow {
            return;
        }
        if bytes.len() > REPLY_BYTES - output.bytes.len()
            || output.bytes.try_reserve_exact(bytes.len()).is_err()
        {
            output.overflow = true;
            return;
        }
        output.bytes.extend_from_slice(bytes);
    })?;
    let result = run(&mut Replies {
        terminal,
        filter: neko_terminal_reply_filter::ReplyOnlyInputFilter::default(),
        output,
        budget: &budget,
        failed: false,
    });
    debug_assert_eq!(
        budget.used.get(),
        0,
        "Ghostty reply authority leaked allocations"
    );
    result
}

impl Replies<'_> {
    fn begin(&mut self) -> Result<()> {
        ensure!(!self.failed, "PTY VT reply authority previously failed");
        self.output.borrow_mut().bytes.clear();
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        let result = (|| {
            ensure!(
                !self.output.borrow().callback_error,
                "PTY VT reply geometry callback failed"
            );
            ensure!(
                !self.budget.exhausted.get(),
                "PTY VT reply memory budget exceeded"
            );
            ensure!(
                !self.output.borrow().overflow,
                "PTY VT reply byte budget exceeded"
            );
            ensure!(
                !self.terminal.vt_processing_error()?,
                "PTY VT reply processing failed"
            );
            Ok(std::mem::take(&mut self.output.borrow_mut().bytes))
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub(super) fn observe(&mut self, live: &[u8]) -> Result<Vec<u8>> {
        self.begin()?;
        if live.len() > INPUT_BYTES {
            self.failed = true;
            bail!("PTY VT reply input chunk budget exceeded");
        }
        self.terminal.vt_write(&self.filter.filter(live));
        self.finish()
    }

    /// Apply the authoritative PTY size before feeding subsequent output. This
    /// also collects Ghostty's in-band resize report when mode 2048 is enabled.
    pub(super) fn resize(&mut self, cols: u16, rows: u16) -> Result<Vec<u8>> {
        self.begin()?;
        if let Err(error) = dimensions(cols, rows) {
            self.failed = true;
            return Err(error);
        }
        match (self.terminal.cols(), self.terminal.rows()) {
            (Ok(current_cols), Ok(current_rows))
                if (current_cols, current_rows) == (cols, rows) =>
            {
                return self.finish();
            }
            (Ok(_), Ok(_)) => {}
            _ => {
                self.failed = true;
                bail!("PTY VT reply geometry lookup failed");
            }
        }
        if let Err(error) = self.terminal.resize(cols, rows, 0, 0) {
            self.failed = true;
            return Err(error).context("PTY VT reply resize failed");
        }
        self.finish()
    }
}

#[cfg(test)]
#[path = "replies_tests.rs"]
mod tests;
