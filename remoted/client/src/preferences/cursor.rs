//! Cursor colors are terminal data, independent of interface theme tokens.
use anyhow::{Result, ensure};

pub(super) fn validate(value: &str) -> Result<()> {
    ensure!(
        value.len() == 7
            && value.starts_with('#')
            && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit),
        "Cursor color must use #RRGGBB"
    );
    Ok(())
}

pub(super) fn parse(value: &str) -> Option<u32> {
    validate(value).ok()?;
    u32::from_str_radix(&value[1..], 16).ok()
}

fn luminance(rgb: u32) -> f64 {
    let channel = |v: u32| {
        let v = f64::from(v & 255) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb >> 16) + 0.7152 * channel(rgb >> 8) + 0.0722 * channel(rgb)
}

/// Keep the theme's cursor when visible, otherwise use a contrasting foreground.
pub(super) fn visible(cursor: u32, background: u32, foreground: u32) -> u32 {
    let contrast = |rgb| {
        let (a, b) = (luminance(rgb), luminance(background));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    if contrast(cursor) >= 3.0 {
        cursor
    } else if contrast(foreground) >= 3.0 {
        foreground
    } else if contrast(0xffffff) > contrast(0) {
        0xffffff
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_cursor_remains_visible_on_dark_and_light_backgrounds() {
        assert_eq!(visible(0x202020, 0x202020, 0xdddddd), 0xdddddd);
        assert_eq!(visible(0xfafafa, 0xffffff, 0x202020), 0x202020);
        assert_eq!(visible(0xffcc00, 0x202020, 0xffffff), 0xffcc00);
        assert_eq!(parse("#12aBef"), Some(0x12abef));
        for value in ["red", "#abc", "#1234567", "#zzzzzz", " #ffffff"] {
            assert!(validate(value).is_err());
        }
    }
}
