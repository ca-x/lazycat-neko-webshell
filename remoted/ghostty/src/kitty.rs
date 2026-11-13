//! Bounded Kitty image snapshots; the terminal remains the owner of placement semantics.

use base64::{Engine, engine::general_purpose::STANDARD};
use libghostty_vt::{
    Terminal,
    alloc::Allocator,
    error::Error,
    kitty::graphics::{self, DecodePng, DecodedImage, ImageFormat, RustPngDecoder},
    render::{ImageIterator, ImageLayer, ImageProtocol, Snapshot},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

/// Maximum decoded bytes for one image (four million RGBA pixels).
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRANSFER_BYTES: usize = 4 * 1024 * 1024;
const MAX_STORAGE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 48 * 1024 * 1024;
const MAX_PLACEMENTS: usize = 256;

/// Install the process PNG decoder before constructing any application Ghostty terminal.
///
/// Every renderer and server authority constructor passes through this gate. Concurrent
/// constructors wait for the one installation before entering any other native call.
///
/// # Errors
/// Returns a native error if the decoder hook cannot be installed.
pub fn initialize_graphics() -> Result<(), Error> {
    static INITIALIZED: OnceLock<Result<(), Error>> = OnceLock::new();
    *INITIALIZED.get_or_init(|| {
        // SAFETY: All app-owned Ghostty constructors call this gate before any
        // native operation, including the server authority. It is never replaced.
        unsafe { graphics::set_png_decoder(Some(Box::new(BoundedPng))) }
    })
}

struct BoundedPng;

impl DecodePng for BoundedPng {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        let stripped = png_payload(data)?;
        RustPngDecoder::new().decode_png(alloc, &stripped)
    }
}

// Exclude ancillary metadata before invoking the decoder: compressed text/ICC
// profiles must not allocate outside the image budget. Transparency is retained.
fn png_payload(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() > MAX_TRANSFER_BYTES || data.get(..8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    if data.get(12..16)? != b"IHDR" || data.get(8..12)? != 13_u32.to_be_bytes() {
        return None;
    }
    let width = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
    rgba_len(width, height)?;
    let mut output = Vec::new();
    output.try_reserve_exact(data.len()).ok()?;
    output.extend_from_slice(&data[..8]);
    let mut offset = 8_usize;
    while offset < data.len() {
        let len = u32::from_be_bytes(data.get(offset..offset.checked_add(4)?)?.try_into().ok()?);
        let end = offset
            .checked_add(12)?
            .checked_add(usize::try_from(len).ok()?)?;
        let chunk = data.get(offset..end)?;
        let kind = chunk.get(4..8)?;
        if kind[0] & 0x20 == 0 || kind == b"tRNS" {
            output.extend_from_slice(chunk);
        }
        offset = end;
        if kind == b"IEND" {
            return Some(output);
        }
    }
    None
}

fn rgba_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return None;
    }
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)
        .filter(|len| *len <= MAX_IMAGE_BYTES)
}

/// Kitty's order relative to terminal cell backgrounds and text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// Draw before explicit cell backgrounds.
    BelowBackground,
    /// Draw after cell backgrounds and before glyphs.
    BelowText,
    /// Draw after glyphs and the cursor.
    AboveText,
}

/// Shared pixels and an encoded canvas source, cached by native content generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Native image identifier.
    pub id: u32,
    /// Changes whenever native pixel contents change.
    pub generation: u64,
    /// Full decoded image width.
    pub width: u32,
    /// Full decoded image height.
    pub height: u32,
    /// Straight RGBA8 pixels, row major.
    pub rgba: Arc<[u8]>,
    /// Bounded PNG data URI for NSH's URI-only canvas image source.
    pub data_uri: Arc<str>,
}

/// One native image placement in global scrollback pixel coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Shared image contents.
    pub image: Arc<Image>,
    /// Global horizontal pixel origin, possibly negative.
    pub x: i64,
    /// Global vertical pixel origin, including the captured first row.
    pub y: i64,
    /// Destination width in native pixels.
    pub width: u32,
    /// Destination height in native pixels.
    pub height: u32,
    /// Source x, y, width and height in image pixels.
    pub source: [u32; 4],
    /// Cell pixel width used by the native placement calculation.
    pub cell_width: u32,
    /// Cell pixel height used by the native placement calculation.
    pub cell_height: u32,
    /// Compositing layer.
    pub layer: Layer,
    /// Ordering within the compositing layer.
    pub z: i32,
}

pub(crate) fn configure(terminal: &mut Terminal<'_, '_>) -> Result<(), Error> {
    terminal.set_kitty_image_storage_limit(MAX_STORAGE_BYTES)?;
    terminal.set_apc_max_bytes_kitty(Some(MAX_TRANSFER_BYTES))?;
    terminal.set_kitty_image_from_file_allowed(false)?;
    terminal.set_kitty_image_temp_file_dir(None)?;
    terminal.set_kitty_image_from_shared_mem_allowed(false)?;
    Ok(())
}

pub(crate) struct Images {
    iterator: ImageIterator<'static>,
    cache: BTreeMap<u64, Arc<Image>>,
}

impl Images {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(Self {
            iterator: ImageIterator::new()?,
            cache: BTreeMap::new(),
        })
    }

    pub(crate) fn snapshot(
        &mut self,
        terminal: &Terminal<'static, 'static>,
        snapshot: &Snapshot<'static, '_>,
        first_row: usize,
        cw: u32,
        ch: u32,
    ) -> Result<Vec<Placement>, Error> {
        let Some(graphics) = terminal.kitty_graphics()? else {
            self.cache.clear();
            return Ok(Vec::new());
        };
        let mut iteration = self.iterator.update(snapshot)?;
        let mut output = Vec::new();
        let mut live = BTreeSet::new();
        let mut frame_bytes = 0_usize;
        while let Some(placement) = iteration.next() {
            if output.len() == MAX_PLACEMENTS {
                break;
            }
            if placement.protocol()? != ImageProtocol::Kitty {
                continue;
            }
            let generation = placement.image_generation()?;
            if !self.cache.contains_key(&generation) {
                let Some(native) = graphics.image(placement.image_id()?) else {
                    continue;
                };
                let Some(image) = read_image(native)? else {
                    continue;
                };
                let cost = image.rgba.len() + image.data_uri.len();
                if cost > MAX_CACHE_BYTES || frame_bytes.saturating_add(cost) > MAX_CACHE_BYTES {
                    continue;
                }
                while self
                    .cache
                    .values()
                    .map(|image| image.rgba.len() + image.data_uri.len())
                    .sum::<usize>()
                    .saturating_add(cost)
                    > MAX_CACHE_BYTES
                {
                    self.cache.pop_first();
                }
                self.cache.insert(generation, Arc::new(image));
            }
            let Some(image) = self.cache.get(&generation).cloned() else {
                continue;
            };
            if live.insert(generation) {
                frame_bytes = frame_bytes.saturating_add(image.rgba.len() + image.data_uri.len());
                if frame_bytes > MAX_CACHE_BYTES {
                    continue;
                }
            }
            let source = [
                placement.source_x()?,
                placement.source_y()?,
                placement.source_width()?,
                placement.source_height()?,
            ];
            if source[2] == 0
                || source[3] == 0
                || source[0]
                    .checked_add(source[2])
                    .is_none_or(|right| right > image.width)
                || source[1]
                    .checked_add(source[3])
                    .is_none_or(|bottom| bottom > image.height)
            {
                continue;
            }
            output.push(Placement {
                image,
                x: i64::from(placement.viewport_col()?) * i64::from(cw)
                    + i64::from(placement.cell_offset_x()?),
                y: i64::try_from(first_row)
                    .map_err(|_| Error::InvalidValue)?
                    .checked_mul(i64::from(ch))
                    .and_then(|y| {
                        y.checked_add(i64::from(placement.viewport_row().ok()?) * i64::from(ch))
                    })
                    .and_then(|y| y.checked_add(i64::from(placement.cell_offset_y().ok()?)))
                    .ok_or(Error::InvalidValue)?,
                width: placement.pixel_width()?,
                height: placement.pixel_height()?,
                source,
                cell_width: cw,
                cell_height: ch,
                layer: match placement.layer()? {
                    ImageLayer::BelowBackground => Layer::BelowBackground,
                    ImageLayer::BelowText => Layer::BelowText,
                    ImageLayer::AboveText => Layer::AboveText,
                },
                z: placement.z()?,
            });
        }
        self.cache.retain(|generation, _| live.contains(generation));
        output.sort_by_key(|placement| (placement.layer, placement.z));
        Ok(output)
    }
}

fn read_image(image: graphics::Image<'_>) -> Result<Option<Image>, Error> {
    let (width, height) = (image.width()?, image.height()?);
    let Some(len) = rgba_len(width, height) else {
        return Ok(None);
    };
    let Some(data) = image.data()? else {
        return Ok(None);
    };
    let bpp = match image.format()? {
        ImageFormat::Rgba => 4,
        ImageFormat::Rgb => 3,
        ImageFormat::GrayAlpha => 2,
        ImageFormat::Gray => 1,
        _ => return Ok(None),
    };
    if data.len() != len / 4 * bpp {
        return Ok(None);
    }
    let mut rgba = Vec::new();
    if rgba.try_reserve_exact(len).is_err() {
        return Ok(None);
    }
    for pixel in data.chunks_exact(bpp) {
        let color = match bpp {
            4 => [pixel[0], pixel[1], pixel[2], pixel[3]],
            3 => [pixel[0], pixel[1], pixel[2], 255],
            2 => [pixel[0], pixel[0], pixel[0], pixel[1]],
            _ => [pixel[0], pixel[0], pixel[0], 255],
        };
        rgba.extend_from_slice(&color);
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let Ok(mut writer) = encoder.write_header() else {
            return Ok(None);
        };
        if writer.write_image_data(&rgba).is_err() || writer.finish().is_err() {
            return Ok(None);
        }
    }
    if encoded.len() > MAX_IMAGE_BYTES + 65536 {
        return Ok(None);
    }
    let data_uri = format!("data:image/png;base64,{}", STANDARD.encode(&encoded));
    Ok(Some(Image {
        id: image.id()?,
        generation: image.generation()?,
        width,
        height,
        rgba: rgba.into(),
        data_uri: data_uri.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Emulator;

    const WHITE: &[u8] = b"\x1b_Ga=T,f=24,s=2,v=2,i=1,q=2;////////////////\x1b\\";

    #[test]
    fn transmit_replace_crop_layer_and_delete_use_native_placements() {
        let mut terminal = Emulator::new(10, 5, 100).unwrap();
        terminal.feed(WHITE);
        let first = terminal.images(0..5, 10, 20).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(&*first[0].image.rgba, &[255; 16]);
        assert!(
            first[0]
                .image
                .data_uri
                .starts_with("data:image/png;base64,")
        );
        let again = terminal.images(0..5, 10, 20).unwrap();
        assert!(
            Arc::ptr_eq(&first[0].image, &again[0].image),
            "unchanged pixels are encoded once"
        );
        terminal.feed(b"\x1b_Ga=p,i=1,p=2,x=1,y=0,w=1,h=2,c=2,r=2,X=3,Y=4,z=-1,q=2\x1b\\");
        let placements = terminal.images(0..5, 10, 20).unwrap();
        let cropped = placements
            .iter()
            .find(|placement| placement.layer == Layer::BelowText)
            .unwrap();
        assert_eq!(cropped.source, [1, 0, 1, 2]);
        assert_eq!(
            cropped.width, 17,
            "native X offset consumes three pixels of the requested cell span"
        );
        terminal.feed(b"\x1b_Ga=T,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
        let replaced = terminal.images(0..5, 10, 20).unwrap();
        assert!(
            replaced
                .iter()
                .all(|placement| placement.image.generation != first[0].image.generation)
        );
        assert_eq!(&*replaced[0].image.rgba, &[0, 0, 0, 255]);
        terminal.feed(b"\x1b_Ga=d,d=A,q=2\x1b\\");
        assert!(terminal.images(0..5, 10, 20).unwrap().is_empty());
    }

    #[test]
    fn native_virtual_placements_and_scrollback_have_one_global_origin() {
        let mut terminal = Emulator::new(10, 5, 100).unwrap();
        terminal.images(0..5, 10, 10).unwrap();
        terminal.feed(b"\x1b_Ga=T,t=d,f=24,i=1,p=1,s=1,v=2,U=1,c=2,r=1,q=2;////////\x1b\\");
        terminal.feed("\x1b[2;4H\x1b[38;5;1;58;5;1m\u{10EEEE}\u{10EEEE}".as_bytes());
        let placements = terminal.images(0..5, 10, 10).unwrap();
        assert_eq!(placements.len(), 1);
        assert_eq!(
            (
                placements[0].x,
                placements[0].y,
                placements[0].width,
                placements[0].height
            ),
            (38, 10, 5, 10)
        );
        terminal.feed(b"\x1b[5;1H\n\n");
        let history = terminal.images(0..5, 10, 10).unwrap();
        assert_eq!(
            history[0].y, 10,
            "scrollback origin must not be subtracted twice"
        );
        terminal.feed(b"\x1b[?1049h");
        assert!(terminal.images(0..5, 10, 10).unwrap().is_empty());
        terminal.feed(b"\x1b[?1049l");
        assert!(!terminal.images(0..5, 10, 10).unwrap().is_empty());
        terminal.feed(b"\x1b[2J\x1b[3J");
        assert!(terminal.images(0..5, 10, 10).unwrap().is_empty());
    }

    #[test]
    fn tall_client_captures_images_beyond_a_short_readonly_source_viewport() {
        let mut terminal = Emulator::new(20, 10, 200).unwrap();
        terminal.images(0..10, 10, 20).unwrap();
        for row in 0..25 {
            terminal.feed(format!("row-{row}\r\n").as_bytes());
        }
        terminal.feed(b"\x1b_Ga=T,f=24,s=2,v=2,i=1,c=2,r=4,C=1,q=2;////////////////\x1b\\");
        terminal.feed(b"image-row\r\n");
        for row in 26..55 {
            terminal.feed(format!("row-{row}\r\n").as_bytes());
        }
        let cursor = terminal.cursor().unwrap();
        assert!(terminal.images(0..10, 10, 20).unwrap().is_empty());

        let tall = terminal.images(0..40, 10, 20).unwrap();
        assert_eq!(tall.len(), 1, "row 25 is visible in the taller client");
        assert_eq!((tall[0].y, tall[0].height), (25 * 20, 4 * 20));
        let scrolled = terminal.images(20..40, 10, 20).unwrap();
        assert_eq!(
            scrolled, tall,
            "a placement keeps its global scrollback origin"
        );
        assert_eq!(terminal.terminal.rows().unwrap(), 10, "readonly PTY size");
        assert_eq!(terminal.terminal.cols().unwrap(), 20);
        assert_eq!(terminal.cursor().unwrap(), cursor);

        for (row, expected) in [(0, "row-0"), (25, "image-row"), (50, "row-50")] {
            let text: String = terminal
                .cells(row)
                .unwrap()
                .iter()
                .map(|cell| cell.text.as_str())
                .collect();
            assert_eq!(
                text.trim_end(),
                expected,
                "text row mapping after image overscan"
            );
        }
        assert!(terminal.images(0..10, 10, 20).unwrap().is_empty());
        assert_eq!(terminal.images(0..40, 10, 20).unwrap(), tall);
        terminal.feed(b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\");
        assert!(terminal.images(0..40, 10, 20).unwrap().is_empty());
        assert!(
            terminal.take_events().is_empty(),
            "images emit no client replies"
        );
    }

    #[test]
    fn png_chunks_decode_bounded_pixels_without_client_replies() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[20, 40, 60, 128]).unwrap();
            writer.finish().unwrap();
        }
        let payload = STANDARD.encode(&png);
        let split = payload.len() / 8 * 4;
        let mut terminal = Emulator::new(10, 5, 0).unwrap();
        terminal.feed(format!("\x1b_Ga=T,f=100,i=9,m=1;{}\x1b\\", &payload[..split]).as_bytes());
        assert!(terminal.images(0..5, 8, 20).unwrap().is_empty());
        terminal.feed(format!("\x1b_Gm=0;{}\x1b\\", &payload[split..]).as_bytes());
        let placements = terminal.images(0..5, 8, 20).unwrap();
        assert_eq!(&*placements[0].image.rgba, &[20, 40, 60, 128]);
        terminal.feed(b"\x1b_Ga=q,f=24,s=1,v=1,i=88;AAAA\x1b\\\x1b[6n\x1b[c");
        assert!(
            terminal.take_events().is_empty(),
            "client image/query parsing has no PTY reply callback"
        );
        assert!(!terminal.processing_error().unwrap());
    }

    #[test]
    fn oversized_or_truncated_png_is_rejected_before_decoding() {
        assert_eq!(rgba_len(8192, 8192), None);
        assert_eq!(rgba_len(u32::MAX, u32::MAX), None);
        assert!(png_payload(b"not png").is_none());
        let mut data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec();
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        data.extend_from_slice(&1_u32.to_be_bytes());
        assert!(png_payload(&data).is_none());
        assert!(png_payload(&vec![0; MAX_TRANSFER_BYTES + 1]).is_none());
    }

    #[test]
    fn concurrent_wrapper_construction_uses_one_global_decoder_gate() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    let mut terminal = Emulator::new(10, 5, 0).unwrap();
                    terminal.feed(WHITE);
                    assert_eq!(terminal.images(0..5, 8, 20).unwrap().len(), 1);
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn image_snapshots_preserve_text_colors_modes_and_cursor() {
        let mut terminal = Emulator::new(20, 5, 100).unwrap();
        terminal.line(0).unwrap();
        terminal.feed(b"\x1b[31mcolored\x1b[0m\x1b[?2004h");
        let cursor = terminal.cursor().unwrap();
        assert!(terminal.images(0..5, 9, 20).unwrap().is_empty());
        let cells = terminal.cells(0).unwrap();
        assert!(
            cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
                .contains("colored")
        );
        assert_eq!(
            cells[0].style.fg_color,
            crate::StyleColor::Palette(libghostty_vt::color::PaletteIndex(1))
        );
        assert_eq!(terminal.modes().unwrap().1, true);
        assert_eq!(terminal.cursor().unwrap(), cursor);
    }
}
