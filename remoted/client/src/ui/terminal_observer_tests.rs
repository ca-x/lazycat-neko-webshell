//! Behavioral checks for native row composition, observer geometry, and menu authority.

use super::*;
use crate::ui::{controls, terminal_surface::Surface};
use nsh_base::TerminalSize;
use nsh_core::platform::Os;
use nsh_engine::{EngineConfig, Environment, Root};
use nsh_testing::Headless;

#[component]
fn Fixture() -> impl IntoElement {
    let surface = Surface::new();
    surface.buffer.resize(TerminalSize::new(12, 3));
    surface.write("猫e\u{301} tail\r\nnext\r\nlast".into());
    let buffer = surface.buffer.clone();
    let enabled = signal(false);
    let resized = signal(None);
    let pasted = signal(String::new());
    let append = surface.clone();
    let evict = surface.clone();
    let replace = surface.clone();
    view()
        .flex_1()
        .child(controls::action(
            "toggle-input",
            false,
            || false,
            move || enabled.update(|on| *on = !*on),
        ))
        .child(controls::action(
            "append-output",
            false,
            || false,
            move || append.write("\r\nlater".into()),
        ))
        .child(controls::action(
            "evict-history",
            false,
            || false,
            move || {
                // Ghostty's line limit is approximate and prunes whole pages.
                // Leave a bounded page margin and prove the selected row is gone.
                evict.write("\r\nreplacement".repeat(4096));
                let first = evict
                    .buffer
                    .cells(0)
                    .into_iter()
                    .map(|cell| cell.text)
                    .collect::<String>();
                assert!(
                    !first.contains("猫e\u{301} tail"),
                    "fixture did not evict the selected source row"
                );
            },
        ))
        .child(controls::action(
            "replace-output",
            false,
            || false,
            move || {
                replace.reset();
                replace.write("replacement".into());
            },
        ))
        .child(text(move || {
            format!(
                "source={}x{};resize={:?};paste={}",
                buffer.size.get().cols,
                buffer.size.get().rows,
                resized.get(),
                pasted.get()
            )
        }))
        .child(render_interactive(
            surface.buffer,
            resized,
            || false,
            computed(move || enabled.get()),
            move |text| pasted.set(text),
        ))
}

fn app(os: Os) -> Headless {
    let environment = Environment {
        os,
        ..Environment::default()
    };
    Headless::start(EngineConfig {
        environment,
        viewport: Size::new(px(640.0), px(480.0)),
        ..EngineConfig::new(
            Root::of::<Fixture>()
                .tokens(nsh_base::tokens_of(Some(include_str!("theme.json"))).unwrap()),
        )
    })
    .unwrap()
}

#[test]
fn observers_keep_source_dimensions_and_paint_a_bounded_native_viewport() {
    for os in [Os::Linux, Os::Android] {
        let mut app = app(os);
        app.first_render().unwrap();
        assert!(app.shows("source=12x3;resize=None;paste="));
        let nodes = app.screen().layout_nodes();
        let rows: Vec<_> = nodes
            .iter()
            .filter(|node| node.test_id.as_deref() == Some("terminal-grid-row"))
            .collect();
        assert_eq!(rows.len(), 1, "one native viewport canvas on {os:?}");
        for row in &rows {
            assert_eq!(row.kind, nsh_core::ViewKind::Canvas);
            assert!(
                !nodes.iter().any(|node| node.parent == row.id),
                "glyphs must not become child controls"
            );
        }
        assert!(rows[0].bounds.origin.y.0 >= geometry::PADDING);
        assert!(rows[0].bounds.size.height.0 > 0.0);
    }
}

#[test]
fn context_menu_selects_and_copies_graphemes_and_gates_paste() {
    let mut app = app(Os::Linux);
    app.first_render().unwrap();
    app.right_click("id:terminal-selection").unwrap();
    app.wait_for("id:terminal-menu-select-all").unwrap();
    assert!(
        app.press("id:terminal-menu-copy").is_err(),
        "copy requires a selection"
    );
    assert!(
        app.press("id:terminal-menu-paste").is_err(),
        "read-only paste must be disabled"
    );
    app.press("id:terminal-menu-select-all").unwrap();
    app.right_click("id:terminal-selection").unwrap();
    app.press("id:terminal-menu-copy").unwrap();
    assert_eq!(
        app.screen().clipboard(),
        Some("猫e\u{301} tail\nnext\nlast")
    );
    app.press("toggle-input").unwrap();
    app.right_click("id:terminal-selection").unwrap();
    app.press("id:terminal-menu-paste").unwrap();
    app.wait_for("source=12x3;resize=None;paste=猫e\u{301} tail\nnext\nlast")
        .unwrap();
    assert!(app.shows("source=12x3;resize=None;paste=猫e\u{301} tail\nnext\nlast"));
}

#[test]
fn desktop_selection_survives_click_cancellation_until_the_physical_release() {
    let mut app = app(Os::Linux);
    app.first_render().unwrap();
    let target = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-selection"))
        .unwrap();
    let press = PressEvent {
        position: Point::new(px(0.0), px(0.0)),
        page_position: target.bounds.origin,
        clicks: 1,
        ..Default::default()
    };
    use nsh_core::{EventKind, EventPayload, MouseButton};
    for kind in [EventKind::PressDown, EventKind::PressUp] {
        app.engine()
            .control()
            .dispatch_event(target.id, kind, EventPayload::Press(press));
    }
    let position = Point::new(px(24.0), px(25.0));
    let mouse = MouseEvent::new(
        position,
        target.bounds.origin + position,
        Default::default(),
        Some(MouseButton::Left),
    );
    app.engine().control().dispatch_event(
        target.id,
        EventKind::MouseMove,
        EventPayload::Mouse(mouse),
    );
    app.wait_for("复制").unwrap();
    app.right_click("id:terminal-selection").unwrap();
    app.press("id:terminal-menu-copy").unwrap();
    assert!(
        app.screen()
            .clipboard()
            .is_some_and(|text| !text.is_empty())
    );
}

#[test]
fn android_long_press_selects_a_complete_native_word_and_copy_uses_the_clipboard() {
    let mut app = app(Os::Android);
    app.first_render().unwrap();
    let target = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-selection"))
        .unwrap();
    let position = Point::new(px(4.0), px(3.0));
    let held = PressEvent {
        position,
        page_position: target.bounds.origin + position,
        ..Default::default()
    };
    app.engine().control().dispatch_event(
        target.id,
        nsh_core::EventKind::LongPress,
        nsh_core::EventPayload::Press(held),
    );
    app.wait_for("复制").unwrap();
    app.press("复制").unwrap();
    assert_eq!(app.screen().clipboard(), Some("猫e\u{301}"));
    assert!(
        !app.shows("全选"),
        "a context sheet must not steal the hold-and-drag gesture"
    );
    assert!(app.shows("source=12x3;resize=None;paste="));
}

#[test]
fn output_outside_a_finalized_selection_preserves_copy_but_eviction_and_reset_invalidate_it() {
    for change in ["evict-history", "replace-output"] {
        let mut app = app(Os::Linux);
        app.first_render().unwrap();
        app.right_click("id:terminal-selection").unwrap();
        app.press("id:terminal-menu-select-all").unwrap();
        app.press("append-output").unwrap();
        app.right_click("id:terminal-selection").unwrap();
        app.press("id:terminal-menu-copy").unwrap();
        assert_eq!(
            app.screen().clipboard(),
            Some("猫e\u{301} tail\nnext\nlast"),
            "unselected output must keep the original selection"
        );
        assert!(
            app.screen()
                .matches("id:terminal-selection-actions")
                .unwrap(),
            "unselected output must keep the selection actions visible"
        );
        app.press(change).unwrap();
        // Feeding the page margin can keep the debug VT busy beyond five seconds.
        app.wait_until(
            &format!(
                "{change}: selection actions disappear after the selected source cells disappear"
            ),
            std::time::Duration::from_secs(30),
            |screen| matches!(screen.matches("id:terminal-selection-actions"), Ok(false)),
        )
        .unwrap();
        app.right_click("id:terminal-selection").unwrap();
        app.wait_until(
            "Copy unavailable after the selected source cells disappear",
            std::time::Duration::from_secs(5),
            |screen| matches!(screen.press_target("id:terminal-menu-copy"), Ok(None)),
        )
        .unwrap();
        assert!(
            app.press("id:terminal-menu-copy").is_err(),
            "{change} must not copy different text under an old selection"
        );
        assert_eq!(
            app.screen().clipboard(),
            Some("猫e\u{301} tail\nnext\nlast")
        );
    }
}

#[component]
fn ImageLayersFixture() -> impl IntoElement {
    let surface = Surface::new();
    surface.buffer.resize(TerminalSize::new(12, 8));
    surface.write(
        concat!(
            "\x1b[?25l\x1b[41mBG\x1b[0m\x1b[1;1H",
            "\x1b_Ga=T,f=24,s=1,v=1,c=3,r=4,C=1,z=-2147483648,i=1,q=2;AQID\x1b\\",
            "\x1b_Ga=T,f=24,s=1,v=1,c=3,r=4,C=1,z=-1,i=2,q=2;AQID\x1b\\",
            "\x1b_Ga=T,f=24,s=1,v=1,c=3,r=4,C=1,z=0,i=3,q=2;AQID\x1b\\",
        )
        .into(),
    );
    render(surface.buffer, signal(None), || false)
}

#[test]
fn images_cross_rows_once_in_native_layer_order_without_clip_seams() {
    use nsh_core::canvas::CanvasOp;
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(640.0), px(480.0)),
        ..EngineConfig::new(Root::of::<ImageLayersFixture>())
    })
    .unwrap();
    let canvases = test_support::Canvases::observe(&mut app);
    app.first_render().unwrap();
    let rows = canvases.rows(&app);
    assert_eq!(rows.len(), 1);
    let drawing = &rows[0].1;
    let images: Vec<_> = drawing
        .ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| matches!(op, CanvasOp::DrawImage { .. }).then_some(i))
        .collect();
    assert_eq!(images.len(), 3, "one draw per placement across all rows");
    let background = drawing
        .ops
        .iter()
        .position(|op| matches!(op, CanvasOp::FillRect(_)))
        .unwrap();
    let glyph = drawing
        .ops
        .iter()
        .position(|op| matches!(op, CanvasOp::FillText { .. }))
        .unwrap();
    assert!(images[0] < background && background < images[1]);
    assert!(images[1] < glyph && glyph < images[2]);
}
