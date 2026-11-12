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
    let cursor = emulator.cursor().unwrap();
    emulator.cells(0).unwrap();
    emulator.cells(1).unwrap();
    emulator.line(0).unwrap();
    assert_eq!(emulator.cursor().unwrap(), cursor);
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

#[test]
fn native_cells_keep_ascii_columns_cjk_and_complete_combining_clusters() {
    let mut emulator = Emulator::new(20, 2, 0).unwrap();
    for byte in " A中e\u{301}文!".as_bytes() {
        emulator.feed(&[*byte]);
    }
    let runs = emulator.line(0).unwrap();
    let cells = emulator.cells(0).unwrap();
    assert_eq!(
        cells[..6]
            .iter()
            .map(|cell| (cell.column, cell.width, cell.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, 1, " "),
            (1, 1, "A"),
            (2, 2, "中"),
            (4, 1, "e\u{301}"),
            (5, 2, "文"),
            (7, 1, "!"),
        ]
    );
    assert_eq!(cells.len(), 18);
    assert_eq!(cells.last().unwrap().column, 19);
    assert!(
        cells
            .windows(2)
            .all(|pair| { pair[1].column == pair[0].column + pair[0].width })
    );
    assert_eq!(
        cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>(),
        runs.iter().map(|run| run.text.as_str()).collect::<String>()
    );
    assert_eq!(emulator.line(0).unwrap(), runs);
    assert!(emulator.cells(usize::MAX).unwrap().is_empty());
}

#[test]
fn native_cells_preserve_zwj_boundaries_with_mode_2027_on_and_off() {
    let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
    for clustered in [false, true] {
        let mut emulator = Emulator::new(20, 2, 0).unwrap();
        emulator.feed(if clustered {
            b"\x1b[?2027h"
        } else {
            b"\x1b[?2027l"
        });
        emulator.feed(format!("{family}A").as_bytes());
        let cells = emulator.cells(0).unwrap();
        let end = cells.iter().position(|cell| cell.text == "A").unwrap();
        let columns: Vec<_> = cells[..=end]
            .iter()
            .map(|cell| (cell.column, cell.width))
            .collect();
        if clustered {
            assert_eq!(columns, vec![(0, 2), (2, 1)]);
            assert_eq!(cells[0].text, family);
        } else {
            assert_eq!(columns, vec![(0, 2), (2, 2), (4, 2), (6, 2), (8, 1)]);
        }
        assert_eq!(
            cells[..=end]
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>(),
            format!("{family}A")
        );
        assert_eq!(
            emulator.cursor().unwrap(),
            Some((if clustered { 3 } else { 9 }, 0))
        );
        assert_eq!(text(&mut emulator, 0), format!("{family}A"));
    }
}

#[test]
fn native_cells_keep_style_boundaries_wide_tails_and_background_blanks() {
    let mut emulator = Emulator::new(12, 2, 0).unwrap();
    emulator.feed("\x1b[31m中\x1b[1;38;2;12;34;56;48;5;200m文\x1b[0;32mB\x1b[44m\x1b[K".as_bytes());
    let cells = emulator.cells(0).unwrap();
    assert_eq!(
        cells[..4]
            .iter()
            .map(|cell| (cell.column, cell.width, cell.text.as_str()))
            .collect::<Vec<_>>(),
        vec![(0, 2, "中"), (2, 2, "文"), (4, 1, "B"), (5, 1, " ")]
    );
    assert!(matches!(cells[0].style.fg_color, StyleColor::Palette(index) if index.0 == 1));
    assert!(cells[1].style.bold);
    assert!(
        matches!(cells[1].style.fg_color, StyleColor::Rgb(color) if (color.r,color.g,color.b)==(12,34,56))
    );
    assert!(matches!(cells[1].style.bg_color, StyleColor::Palette(index) if index.0 == 200));
    assert!(matches!(cells[2].style.fg_color, StyleColor::Palette(index) if index.0 == 2));
    assert!(matches!(cells[3].style.bg_color, StyleColor::Palette(index) if index.0 == 4));
    assert_eq!(cells.len(), 10);
    assert_eq!(text(&mut emulator, 0), "中文B");
}

#[test]
fn native_cells_keep_the_wide_wrap_head_as_one_blank_slot() {
    let mut emulator = Emulator::new(4, 2, 0).unwrap();
    emulator.feed("ABC中Z".as_bytes());
    let first = emulator.cells(0).unwrap();
    assert_eq!(first.len(), 4);
    assert_eq!(
        (first[3].column, first[3].width, first[3].text.as_str()),
        (3, 1, " ")
    );
    let second = emulator.cells(1).unwrap();
    assert_eq!(
        second
            .iter()
            .map(|cell| (cell.column, cell.width, cell.text.as_str()))
            .collect::<Vec<_>>(),
        vec![(0, 2, "中"), (2, 1, "Z"), (3, 1, " ")]
    );
    assert_eq!(text(&mut emulator, 0), "ABC");
    assert_eq!(text(&mut emulator, 1), "中Z");
}

#[test]
fn native_cells_share_run_cache_through_scrollback_resize_and_alternate_screen() {
    let mut emulator = Emulator::new(12, 2, 100).unwrap();
    emulator.feed(b"first\r\nsecond\r\nthird");
    let cursor = emulator.cursor().unwrap();
    let first = emulator.cells(0).unwrap();
    assert_eq!(first[0].text, "f");
    assert_eq!(text(&mut emulator, 2), "third");
    assert_eq!(emulator.cursor().unwrap(), cursor);
    emulator.feed(b"!");
    assert_eq!(emulator.cells(2).unwrap()[5].text, "!");
    emulator.feed(b"\x1b[?1049h\x1b[HOTHER");
    assert_eq!(emulator.cells(0).unwrap()[0].text, "O");
    emulator.feed(b"\x1b[?1049l");
    assert_eq!(emulator.cells(0).unwrap(), first);
    emulator.resize(20, 3).unwrap();
    for row in 0..emulator.line_count().unwrap() {
        let cells = emulator.cells(row).unwrap();
        assert_eq!(cells.last().unwrap().column, 19);
        assert_eq!(
            cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>(),
            emulator
                .line(row)
                .unwrap()
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        );
    }
}
