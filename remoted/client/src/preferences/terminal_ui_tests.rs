//! Headless integration of actual Ghostty surfaces and the read-only terminal renderer.

use crate::preferences::{
    Appearance, Preferences, Ui, catalog::Catalog, current_context, terminal::color,
};
use crate::ui::{terminal_observer, terminal_surface::Surface};
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn fixture(observe: bool, scrollback: bool) -> impl IntoElement {
    let catalog = Catalog::shared().expect("test catalog");
    let mut saved = Preferences::defaults(catalog);
    saved.appearance = Appearance::Light;
    let ui = Ui::from_loaded(saved).expect("fixture preferences");
    let context = current_context();
    let surface = Surface::new();
    if scrollback {
        surface.write(
            (0..100)
                .map(|index| format!("ROW-{index:03}\r\n"))
                .collect(),
        );
    } else {
        surface.write(
            "PLAIN \x1b[31mRED\x1b[0m \x1b[7mINVERSE\x1b[0m \x1b[38;2;12;34;56mTRUE\x1b[0m".into(),
        );
    }
    let buffer = surface.buffer.clone();
    let raw = surface.buffer.clone();
    let size = signal((80, 24));
    let content = if observe {
        terminal_observer::render(surface.buffer, size).into_element()
    } else {
        provide_context(surface.factory());
        context
            .terminal()
            .component_ref(&surface.reference)
            .flex_1()
            .into_element()
    };
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(
            nsh_base::button()
                .on_press(move || {
                    ui.update(|preferences| {
                        preferences.theme_id = "light".into();
                        preferences.font_size = 20;
                    })
                })
                .child(text("Light terminal")),
        )
        .child(
            nsh_base::button()
                .on_press(move || {
                    ui.update(|preferences| {
                        preferences.theme_id = "catppuccin-mocha".into();
                        preferences.font_size = 14;
                    })
                })
                .child(text("Dark terminal")),
        )
        .child(text(move || {
            format!(
                "geometry={}x{};request={}x{}",
                buffer.size.get().cols,
                buffer.size.get().rows,
                size.get().0,
                size.get().1
            )
        }))
        .child(text(move || {
            format!("selected={}", ui.preferences().theme_id)
        }))
        .child(text(move || {
            format!(
                "raw={:?}",
                raw.line(0)
                    .runs
                    .iter()
                    .find(|run| run.text.contains("RED"))
                    .and_then(|run| run.style.fg)
            )
        }))
        .child(text(move || {
            let theme = ui.theme();
            format!(
                "cursor={};selection={}",
                context.cursor() == color(theme.cursor),
                context.selection()
                    == nsh_core::color::Hsla::from(nsh_core::color::rgb(
                        theme.selection_background
                    ))
            )
        }))
        .child(content)
}

#[component]
fn Observer() -> impl IntoElement {
    fixture(true, false)
}

#[component]
fn Interactive() -> impl IntoElement {
    fixture(false, false)
}

#[component]
fn Scrollback() -> impl IntoElement {
    fixture(true, true)
}

fn start<Component: nsh_core::element::Component<Props = ()>>() -> Headless {
    *crate::platform::STORE.lock().expect("test store") = Default::default();
    Headless::start(EngineConfig::new(Root::of::<Component>().tokens(
        nsh_base::tokens_of(Some(include_str!("../ui/theme.json"))).expect("app tokens"),
    )))
    .expect("headless terminal")
}

fn rgb(value: u32) -> nsh_core::Color {
    nsh_core::color::rgb(value).into()
}

#[test]
fn observer_uses_terminal_defaults_ansi_inverse_font_and_selection_with_light_ui()
-> Result<(), String> {
    let mut app = start::<Observer>();
    app.wait_for("terminal-observer")?;
    assert_eq!(app.screen().span_style("PLAIN")?.color, rgb(0xcdd6f4));
    assert_eq!(app.screen().span_style("RED")?.color, rgb(0xf38ba8));
    assert_eq!(app.screen().span_style("INVERSE")?.color, rgb(0x1e1e2e));
    assert_eq!(app.screen().span_style("TRUE")?.color, rgb(0x0c2238));
    assert_eq!(app.screen().span_style("PLAIN")?.font_size.0, 14.0);
    assert!(app.screen().in_selectable_view("text:/^PLAIN/")?);
    app.wait_for("cursor=true;selection=true")?;
    app.press("Light terminal")?;
    app.wait_for("selected=light")?;
    assert_eq!(app.screen().span_style("PLAIN")?.color, rgb(0));
    assert_eq!(app.screen().span_style("RED")?.color, rgb(0xbb0000));
    assert_eq!(app.screen().span_style("INVERSE")?.color, rgb(0xffffff));
    assert_eq!(app.screen().span_style("TRUE")?.color, rgb(0x0c2238));
    assert_eq!(app.screen().span_style("PLAIN")?.font_size.0, 20.0);
    app.wait_for("geometry=80x24;request=80x24")?;
    app.wait_for("raw=Some(Indexed(1))")?;
    app.set_viewport(Size::new(px(280.0), px(500.0)))?;
    app.wait_for("geometry=80x24;request=80x24")?;
    let before = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-observer"))
        .ok_or("observer has no layout")?
        .bounds
        .origin
        .x;
    app.scroll("terminal-observer-horizontal", px(200.0))?;
    let after = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-observer"))
        .ok_or("observer has no layout after scrolling")?
        .bounds
        .origin
        .x;
    assert!(
        after < before,
        "horizontal scroll must move fixed terminal columns"
    );
    app.press("Dark terminal")?;
    app.wait_for("selected=catppuccin-mocha")?;
    assert_eq!(app.screen().span_style("RED")?.color, rgb(0xf38ba8));
    Ok(())
}

#[test]
fn shared_emulator_recolors_existing_ansi_without_new_output() -> Result<(), String> {
    let mut app = start::<Interactive>();
    app.wait_for("text:/PLAIN/")?;
    assert_eq!(app.screen().span_style("RED")?.color, rgb(0xf38ba8));
    app.press("Light terminal")?;
    app.wait_for("selected=light")?;
    assert_eq!(app.screen().span_style("RED")?.color, rgb(0xbb0000));
    app.wait_for("raw=Some(Indexed(1))")?;
    Ok(())
}

#[test]
fn observer_scrolls_real_history_without_resizing_or_exposing_input() -> Result<(), String> {
    let mut app = start::<Scrollback>();
    app.wait_for("terminal-observer")?;
    let before: Vec<_> = app
        .screen()
        .texts()
        .into_iter()
        .filter(|text| text.starts_with("ROW-"))
        .map(str::to_owned)
        .collect();
    app.scroll("terminal-observer", px(-180.0))?;
    let after: Vec<_> = app
        .screen()
        .texts()
        .into_iter()
        .filter(|text| text.starts_with("ROW-"))
        .map(str::to_owned)
        .collect();
    assert_ne!(
        after, before,
        "vertical scroll must reveal different history rows"
    );
    let row = app
        .screen()
        .texts()
        .into_iter()
        .find(|text| text.starts_with("ROW-"))
        .ok_or("no rendered history row")?
        .to_owned();
    assert!(app.screen().in_selectable_view(&row)?);
    assert_eq!(
        app.screen().input_text("远程终端；取得控制权后可输入")?,
        None
    );
    app.wait_for("geometry=80x24;request=80x24")?;
    app.scroll("terminal-observer", px(180.0))?;
    app.set_viewport(Size::new(px(280.0), px(500.0)))?;
    app.wait_for("geometry=80x24;request=80x24")?;
    Ok(())
}
