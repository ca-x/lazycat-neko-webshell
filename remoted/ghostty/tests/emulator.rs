use neko_ghostty::{Emulator, Event, StyleColor, Underline};

fn text(emulator: &mut Emulator, row: usize) -> String {
    emulator
        .line(row)
        .unwrap()
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn scrollback_alternate_screen_and_resize() {
    let mut emulator = Emulator::new(12, 2, 100).unwrap();
    emulator.feed(b"first\r\nsecond\r\nthird");
    assert_eq!(emulator.line_count().unwrap(), 3);
    assert_eq!(text(&mut emulator, 0), "first");
    assert_eq!(text(&mut emulator, 2), "third");
    let cursor = emulator.cursor().unwrap();
    assert_eq!(text(&mut emulator, 0), "first");
    assert_eq!(emulator.cursor().unwrap(), cursor);
    emulator.feed(b"!");
    assert_eq!(text(&mut emulator, 2), "third!");
    emulator.feed(b"\x1b[?1049h\x1b[HOTHER");
    assert_eq!(emulator.line_count().unwrap(), 2);
    assert!(text(&mut emulator, 0).contains("OTHER"));
    emulator.feed(b"\x1b[?1049l");
    assert_eq!(text(&mut emulator, 0), "first");
    emulator.resize(20, 3).unwrap();
    let content = (0..emulator.line_count().unwrap())
        .map(|index| text(&mut emulator, index))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(content.contains("first") && content.contains("third"));
    assert!(emulator.line(usize::MAX).unwrap().is_empty());
}

#[test]
fn cjk_combining_and_split_utf8() {
    let mut emulator = Emulator::new(20, 2, 0).unwrap();
    for byte in "中a\u{301}文!".as_bytes() {
        emulator.feed(&[*byte]);
    }
    assert_eq!(text(&mut emulator, 0), "中a\u{301}文!");
    assert_eq!(emulator.cursor().unwrap(), Some((6, 0)));
    emulator.feed(b"\x1b[?25l");
    assert_eq!(emulator.cursor().unwrap(), None);
    emulator.feed(b"\x1b[?25h");
    assert_eq!(emulator.cursor().unwrap(), Some((6, 0)));
}

#[test]
fn colors_styles_and_palette_changes() {
    let mut emulator = Emulator::new(20, 2, 0).unwrap();
    emulator.feed(b"\x1b[1;2;3;4;7;8;9;38;2;12;34;56;48;5;200mX");
    let run = &emulator.line(0).unwrap()[0];
    assert_eq!(run.text, "X");
    assert!(
        run.style.bold
            && run.style.faint
            && run.style.italic
            && run.style.inverse
            && run.style.invisible
            && run.style.strikethrough
    );
    assert_ne!(run.style.underline, Underline::None);
    assert!(
        matches!(run.style.fg_color, StyleColor::Rgb(color) if (color.r,color.g,color.b)==(12,34,56))
    );
    assert!(matches!(run.style.bg_color, StyleColor::Palette(index) if index.0 == 200));
    emulator.feed(b"\x1b[0m\r\x1b[31mR\x1b]4;1;rgb:01/02/03\x1b\\");
    assert!(
        matches!(emulator.line(0).unwrap()[0].style.fg_color, StyleColor::Rgb(color) if (color.r,color.g,color.b)==(1,2,3))
    );
}

#[test]
fn modes_events_and_queries_have_no_reply_channel() {
    let mut emulator = Emulator::new(20, 2, 100).unwrap();
    emulator.feed(b"\x1b[?1h\x1b[?2004h\x1b]0;title\x07\x07\x1b[5n\x1b[6n\x1b[c\x1b]10;?\x1b\\");
    assert_eq!(emulator.modes().unwrap(), (true, true));
    assert_eq!(
        emulator.take_events(),
        vec![Event::Title("title".into()), Event::Bell]
    );
    assert!(emulator.take_events().is_empty());
    emulator.feed(b"\x1b[?1l\x1b[?2004l");
    assert_eq!(emulator.modes().unwrap(), (false, false));
    emulator.feed(b"\x1bc");
    assert_eq!(emulator.modes().unwrap(), (false, false));
}

#[test]
fn osc_default_colors_invalidate_rendered_lines() {
    let mut emulator = Emulator::new(20, 2, 0).unwrap();
    emulator.feed(b"X");
    emulator.line(0).unwrap();
    emulator.feed(b"\x1b]10;rgb:01/02/03\x1b\\\x1b]11;rgb:04/05/06\x1b\\");
    let run = &emulator.line(0).unwrap()[0];
    assert!(
        matches!(run.style.fg_color, StyleColor::Rgb(color) if (color.r,color.g,color.b)==(1,2,3))
    );
    assert!(
        matches!(run.style.bg_color, StyleColor::Rgb(color) if (color.r,color.g,color.b)==(4,5,6))
    );
}

#[test]
fn ansi_indices_survive_for_theme_palette_and_background_only_cells() {
    let mut emulator = Emulator::new(20, 2, 0).unwrap();
    emulator.feed(b"\x1b[31;48;5;200mX\x1b[44m\x1b[K");
    let runs = emulator.line(0).unwrap();
    assert!(matches!(runs[0].style.fg_color, StyleColor::Palette(index) if index.0 == 1));
    assert!(matches!(runs[0].style.bg_color, StyleColor::Palette(index) if index.0 == 200));
    assert!(matches!(runs[1].style.bg_color, StyleColor::Palette(index) if index.0 == 4));
}
