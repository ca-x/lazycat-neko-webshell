//! Headless integration of actual Ghostty surfaces and the read-only terminal renderer.

use crate::preferences::{
    Appearance, Preferences, Ui, catalog::Catalog, current_context, terminal::color,
};
use crate::ui::terminal_observer::test_support::Canvases;
use crate::ui::{terminal_observer, terminal_surface::Surface};
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Environment, Root};
use nsh_testing::Headless;

fn fixture(scrollback: bool) -> impl IntoElement {
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
    let size = signal(None::<(u16, u16)>);
    let content = terminal_observer::render(surface.buffer, size, || false);
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
                size.get().unwrap_or((80, 24)).0,
                size.get().unwrap_or((80, 24)).1
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
    fixture(false)
}

#[component]
fn Scrollback() -> impl IntoElement {
    fixture(true)
}

fn start<Component: nsh_core::element::Component<Props = ()>>() -> (Headless, Canvases) {
    *crate::platform::STORE.lock().expect("test store") = Default::default();
    let mut environment = Environment::default();
    environment.os = nsh_core::platform::Os::Linux;
    environment.idiom = nsh_core::platform::Idiom::Desktop;
    let mut app = Headless::start(EngineConfig {
        environment,
        ..EngineConfig::new(Root::of::<Component>().tokens(
            nsh_base::tokens_of(Some(include_str!("../ui/theme.json"))).expect("app tokens"),
        ))
    })
    .expect("headless terminal");
    let canvases = Canvases::observe(&mut app);
    (app, canvases)
}

fn rgb(value: u32) -> nsh_core::Color {
    nsh_core::color::rgb(value).into()
}

#[test]
fn observer_uses_terminal_defaults_ansi_inverse_font_and_selection_with_light_ui()
-> Result<(), String> {
    let (mut app, canvases) = start::<Observer>();
    app.wait_for("id:terminal-observer")?;
    assert_eq!(canvases.glyph(&app, "P").color, rgb(0xcdd6f4));
    assert_eq!(canvases.glyph(&app, "R").color, rgb(0xf38ba8));
    assert_eq!(canvases.glyph(&app, "V").color, rgb(0x1e1e2e));
    assert_eq!(canvases.glyph(&app, "T").color, rgb(0x0c2238));
    assert_eq!(canvases.glyph(&app, "P").font.size.0, 14.0);
    assert!(
        canvases
            .glyph(&app, "P")
            .font
            .family
            .unwrap()
            .contains("Maple Mono NF CN")
    );
    let plain = canvases.glyph(&app, "P");
    assert_eq!(canvases.glyph(&app, "R").x - plain.x, 6.0 * plain.width.0);
    assert!(
        canvases.has_fill(&app, rgb(0xcdd6f4), plain.width.0),
        "inverse cells paint their actual background"
    );
    app.wait_for("cursor=true;selection=true")?;
    app.press("Light terminal")?;
    app.wait_for("selected=light")?;
    assert_eq!(canvases.glyph(&app, "P").color, rgb(0));
    assert_eq!(canvases.glyph(&app, "R").color, rgb(0xbb0000));
    assert_eq!(canvases.glyph(&app, "V").color, rgb(0xffffff));
    assert_eq!(canvases.glyph(&app, "T").color, rgb(0x0c2238));
    assert_eq!(canvases.glyph(&app, "P").font.size.0, 20.0);
    app.wait_for("geometry=80x24;request=80x24")?;
    app.wait_for("raw=Some(Indexed(1))")?;
    app.set_viewport(Size::new(px(280.0), px(500.0)))?;
    let before = canvases.rows(&app)[0].0.origin.x;
    app.scroll("id:terminal-observer-horizontal", px(200.0))?;
    assert!(
        canvases.rows(&app)[0].0.origin.x < before,
        "horizontal scroll moves the source grid"
    );
    app.press("Dark terminal")?;
    app.wait_for("selected=catppuccin-mocha")?;
    assert_eq!(canvases.glyph(&app, "R").color, rgb(0xf38ba8));
    app.right_click("id:terminal-selection")?;
    app.press("id:terminal-menu-select-all")?;
    let theme = Catalog::shared()
        .expect("catalog")
        .find("catppuccin-mocha")
        .expect("theme");
    assert!(
        canvases.has_fill(
            &app,
            rgb(theme.selection_background),
            80.0 * canvases.glyph(&app, "P").width.0
        ),
        "select all paints the theme selection across the source columns"
    );
    app.right_click("id:terminal-selection")?;
    app.press("id:terminal-menu-copy")?;
    assert!(
        app.screen()
            .clipboard()
            .is_some_and(|text| text.starts_with("PLAIN RED INVERSE TRUE"))
    );
    Ok(())
}

#[test]
fn existing_native_cells_recolor_without_new_output() -> Result<(), String> {
    let (mut app, canvases) = start::<Observer>();
    app.wait_for("id:terminal-observer")?;
    assert_eq!(canvases.glyph(&app, "R").color, rgb(0xf38ba8));
    app.press("Light terminal")?;
    app.wait_for("selected=light")?;
    assert_eq!(canvases.glyph(&app, "R").color, rgb(0xbb0000));
    app.wait_for("raw=Some(Indexed(1))")?;
    Ok(())
}

#[test]
fn observer_scrolls_real_history_without_resizing_or_exposing_input() -> Result<(), String> {
    let (mut app, canvases) = start::<Scrollback>();
    app.wait_for("id:terminal-observer")?;
    let before = canvases.text(&app);
    app.scroll("id:terminal-observer", px(-180.0))?;
    let after = canvases.text(&app);
    assert_ne!(
        after, before,
        "vertical scroll must paint different history rows"
    );
    assert!(after.contains("ROW-"));
    assert!(app.screen().matches("id:terminal-selection")?);
    assert_eq!(
        app.screen().input_text("远程终端；取得控制权后可输入")?,
        None
    );
    app.wait_for("geometry=80x24;request=80x24")?;
    app.scroll("id:terminal-observer", px(180.0))?;
    app.set_viewport(Size::new(px(280.0), px(500.0)))?;
    app.wait_for("geometry=80x24;request=80x24")?;
    Ok(())
}
