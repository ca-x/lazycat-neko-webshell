//! Desktop settings in a native child window sharing the application's existing owners.

use super::{connection, state::Ui};
use crate::preferences::{Language, Ui as Preferences};
use nsh_base::{tab, tabs, theme};
use nsh_core::{
    desktop::{WindowAttributes, WindowKind, WindowPosition},
    element::desktop,
    prelude::*,
};

/// Mount beside the main shell; settings never replace or remount its terminals.
pub(super) fn render(ui: Ui) -> impl IntoElement {
    let preferences = use_context::<Preferences>();
    let selected = signal(SettingTab::Terminal);
    popup(ui.navigation.settings, move || {
        view()
            .flex_1()
            .min_h(px(0.0))
            .bg(move || theme().background)
            .text_color(move || theme().foreground)
            .child(tab_list(selected, preferences))
            .children(
                [
                    SettingTab::Terminal,
                    SettingTab::Appearance,
                    SettingTab::Connection,
                    SettingTab::RemoteAccess,
                ]
                .into_iter()
                .map(move |section| {
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .style_when(move || selected.get() != section, |style| style.hidden())
                        .child(match section {
                            SettingTab::Terminal => scroll()
                                .flex_1()
                                .child(view().p_4().child(preferences.map(|preferences| {
                                    super::preferences::render_terminal(preferences).into_element()
                                })))
                                .into_element(),
                            SettingTab::Appearance => scroll()
                                .flex_1()
                                .child(view().p_4().child(preferences.map(|preferences| {
                                    super::preferences::render_appearance(preferences)
                                        .into_element()
                                })))
                                .into_element(),
                            SettingTab::Connection => scroll()
                                .flex_1()
                                .child(view().p_4().child(connection::connection_settings(ui)))
                                .into_element(),
                            SettingTab::RemoteAccess => scroll()
                                .flex_1()
                                .child(view().p_4().child(connection::remote_access_settings(ui)))
                                .into_element(),
                        })
                }),
            )
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingTab {
    Terminal,
    Appearance,
    Connection,
    RemoteAccess,
}

impl SettingTab {
    fn label(self, preferences: Option<Preferences>) -> &'static str {
        let chinese = match preferences
            .map(|preferences| preferences.preferences().language)
            .unwrap_or_default()
        {
            Language::ZhCn => true,
            Language::EnUs => false,
            Language::System => locale().as_str().starts_with("zh"),
        };
        match (self, chinese) {
            (Self::Terminal, false) => "Terminal",
            (Self::Appearance, false) => "Appearance",
            (Self::Connection, false) => "Connection",
            (Self::RemoteAccess, false) => "Remote access",
            (Self::Terminal, true) => "终端",
            (Self::Appearance, true) => "外观",
            (Self::Connection, true) => "连接",
            (Self::RemoteAccess, true) => "远程访问",
        }
    }
}

fn tab_list(selected: Signal<SettingTab>, preferences: Option<Preferences>) -> impl IntoElement {
    tabs()
        .flex_row()
        .gap_1()
        .p_2()
        .border_b_1()
        .border_color(move || theme().border)
        .children(
            [
                SettingTab::Terminal,
                SettingTab::Appearance,
                SettingTab::Connection,
                SettingTab::RemoteAccess,
            ]
            .into_iter()
            .map(move |section| {
                let node = element_ref();
                tab()
                    .element_ref(&node)
                    .selected(move || selected.get() == section)
                    .on_press(move || {
                        selected.set(section);
                        node.focus();
                    })
                    .flex_1()
                    .min_h(px(32.0))
                    .px_2()
                    .py_2()
                    .items_center()
                    .justify_center()
                    .rounded(move || theme().radius)
                    .selected_style(|style| style.bg(move || theme().secondary))
                    .hover(|style| style.bg(move || theme().accent))
                    .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
                    .active(|style| style.opacity(0.75))
                    .child(text(move || section.label(preferences)).text_sm())
            }),
        )
}

fn popup<E: IntoElements>(
    open: Signal<bool>,
    content: impl Fn() -> E + 'static,
) -> impl IntoElement {
    let mut attributes = WindowAttributes::default();
    attributes.min_size = Some(Some(size(px(420.0), px(320.0))));
    attributes.resizable = Some(true);
    desktop::window()
        .id("settings")
        .parent("main")
        .kind(WindowKind::Dialog)
        .title(move || crate::preferences::tr("设置").to_string())
        .modal(false)
        .attributes(attributes)
        .default_position(WindowPosition::ParentCenter)
        .default_size(size(px(680.0), px(720.0)))
        .restore_bounds(true)
        .open(open)
        .on_open_change(move |event| {
            open.set(event.open);
            if !event.open
                && desktop::main_window()
                    .state()
                    .is_some_and(|window| window.visible && !window.minimized)
            {
                desktop::main_window().focus();
            }
        })
        .content(move || {
            let child = desktop::current_window();
            let main_visible = computed(|| {
                desktop::main_window()
                    .state()
                    .map(|main| main.visible && !main.minimized)
            });
            // The native modeless child does not automatically follow a hidden
            // parent. Keep its owner mounted and mirror the main window instead.
            effect(move || {
                if let Some(visible) = main_visible.get() {
                    if visible {
                        child.show();
                    } else {
                        child.hide();
                    }
                }
            });
            content()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::{
        element::Root,
        platform::{Environment, Locale, Os},
    };
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;

    #[component]
    fn Fixture() -> impl IntoElement {
        let open = signal(false);
        let shared = signal(0_u32);
        let input = signal("unfinished terminal input".to_owned());
        let selected = signal(SettingTab::Terminal);
        desktop::set_tray(desktop::tray().on_click(|_| {
            desktop::main_window().show();
            desktop::main_window().restore();
            desktop::main_window().focus();
        }));
        view()
            .child(
                nsh_base::button()
                    .on_press(move || open.set(true))
                    .child(text("Open settings")),
            )
            .child(
                nsh_base::input()
                    .value(input)
                    .label("Terminal input")
                    .on_change(move |value: &str| input.set(value.to_owned())),
            )
            .child(text(move || format!("main={}", shared.get())))
            .child(
                nsh_base::button()
                    .on_press(|| desktop::main_window().hide())
                    .child(text("Hide main for tray")),
            )
            .child(popup(open, move || {
                view()
                    .child(tab_list(selected, None))
                    .child(text(move || format!("tab={:?}", selected.get())))
                    .child(text(move || format!("settings={}", shared.get())))
                    .child(
                        nsh_base::button()
                            .on_press(move || shared.update(|value| *value += 1))
                            .child(text("Change shared setting")),
                    )
            }))
    }

    #[test]
    fn native_settings_window_shares_state_and_preserves_the_main_surface() -> Result<(), String> {
        let mut app = Headless::start(EngineConfig {
            environment: Environment {
                os: Os::Linux,
                locale: Locale::EN_US,
                ..Default::default()
            },
            ..EngineConfig::new(Root::of::<Fixture>())
        })
        .map_err(|error| error.to_string())?;
        app.wait_for("main=0")?;
        assert!(app.window_state("settings")?.is_none());
        app.press("Open settings")?;
        app.wait_for("settings=0")?;
        let child = app
            .window_state("settings")?
            .expect("native child window is open");
        assert!(child.visible);
        assert_eq!(child.bounds.size.width.0, 680.0);
        app.press("Change shared setting")?;
        app.wait_for("main=1")?;
        app.wait_for("settings=1")?;
        app.press_window_close("settings")?;
        assert!(app.window_state("settings")?.is_none());
        assert!(app.window_state("main")?.unwrap().visible);
        app.wait_for("main=1")?;
        assert_eq!(
            app.screen().input_text("label:Terminal input")?,
            Some("unfinished terminal input")
        );
        app.press("Open settings")?;
        app.wait_for("settings=1")?;
        assert!(app.window_state("settings")?.unwrap().visible);
        app.press("Appearance")?;
        app.wait_for("tab=Appearance")?;
        app.press_key("right")?;
        app.wait_for("tab=Connection")?;
        app.press_key("end")?;
        app.wait_for("tab=RemoteAccess")?;
        app.press_window_close("settings")?;
        app.press("Open settings")?;
        app.wait_for("tab=RemoteAccess")?;
        app.press("Hide main for tray")?;
        assert!(!app.window_state("main")?.unwrap().visible);
        assert!(
            !app.window_state("settings")?.unwrap().visible,
            "settings follows tray-hidden main"
        );
        app.press_window_close("settings")?;
        assert!(
            !app.window_state("main")?.unwrap().visible,
            "closing hidden settings must not focus/show main"
        );
        let click = app
            .screen()
            .commands()
            .iter()
            .find_map(|(_, command)| match command {
                nsh_core::Command::Desktop(command) => match &**command {
                    nsh_core::desktop::DesktopCommand::SetTray(Some(tray)) => Some(tray.click),
                    _ => None,
                },
                _ => None,
            })
            .expect("test tray restore action");
        app.engine().control().dispatch_event(
            nsh_core::ROOT_ID,
            nsh_core::EventKind::Menu,
            nsh_core::EventPayload::Map(vec![(
                "id",
                nsh_core::PayloadValue::Number(f64::from(click)),
            )]),
        );
        app.wait_for("main=1")?;
        assert!(app.window_state("main")?.unwrap().visible);
        app.press("Open settings")?;
        app.wait_for("settings=1")?;
        Ok(())
    }
}
