//! Mountable native preference view; connection and navigation state stay with the host.

use super::controls::{action, hint};
use crate::preferences::{Appearance, Language, MAX_FONT_SIZE, MIN_FONT_SIZE, SaveState, Ui};
use neko_remoted_protocol::messages::SessionBackend;
use nsh_base::{SelectEntry, SelectItem, select, theme};
use nsh_core::{platform::Os, prelude::*};

/// Render the app-lifetime preference handle; mount inside the host's scrolling settings page.
pub fn render(ui: Ui) -> impl IntoElement {
    view()
        .gap_4()
        .w_full()
        .text_color(move || theme().foreground)
        .child(
            text(move || ui.tr("preferences.title"))
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .role(AccessibilityRole::Header),
        )
        .child(choice(
            ui,
            "preferences.theme",
            move || {
                ui.catalog()
                    .themes
                    .iter()
                    .map(|theme| SelectItem::new(theme.id.clone(), theme.label.clone()))
                    .collect()
            },
            move || ui.preferences().theme_id,
            move || ui.theme().label.clone(),
            move |value| ui.update(|preferences| preferences.theme_id = value),
        ))
        .child(hint(move || {
            let theme = ui.theme();
            let name = theme.ghostty_name.as_deref().unwrap_or(&theme.label);
            if theme.origin == "restty" {
                format!("Restty {} · {name}", ui.catalog().restty_version)
            } else {
                format!("Inkstone · {name}")
            }
        }))
        .child(
            view()
                .gap_2()
                .child(text(move || ui.tr("preferences.font_size")).text_sm())
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .items_center()
                        .child(action(
                            move || ui.tr("preferences.smaller"),
                            false,
                            move || !ui.editable() || ui.preferences().font_size <= MIN_FONT_SIZE,
                            move || {
                                ui.update(|preferences| {
                                    preferences.font_size = preferences.font_size.saturating_sub(1)
                                })
                            },
                        ))
                        .child(
                            text(move || ui.preferences().font_size.to_string())
                                .text_base()
                                .tabular_nums(),
                        )
                        .child(action(
                            move || ui.tr("preferences.larger"),
                            false,
                            move || !ui.editable() || ui.preferences().font_size >= MAX_FONT_SIZE,
                            move || {
                                ui.update(|preferences| {
                                    preferences.font_size = preferences.font_size.saturating_add(1)
                                })
                            },
                        )),
                ),
        )
        .child(choice(
            ui,
            "preferences.appearance",
            move || {
                [
                    ("system", "preferences.system"),
                    ("light", "preferences.light"),
                    ("dark", "preferences.dark"),
                ]
                .into_iter()
                .map(|(value, key)| SelectItem::new(value, ui.tr(key)))
                .collect()
            },
            move || {
                match ui.preferences().appearance {
                    Appearance::System => "system",
                    Appearance::Light => "light",
                    Appearance::Dark => "dark",
                }
                .into()
            },
            move || {
                ui.tr(match ui.preferences().appearance {
                    Appearance::System => "preferences.system",
                    Appearance::Light => "preferences.light",
                    Appearance::Dark => "preferences.dark",
                })
                .into()
            },
            move |value| {
                ui.update(|preferences| {
                    preferences.appearance = match value.as_str() {
                        "light" => Appearance::Light,
                        "dark" => Appearance::Dark,
                        _ => Appearance::System,
                    }
                })
            },
        ))
        .child(choice(
            ui,
            "preferences.language",
            move || {
                vec![
                    SelectItem::new("system", ui.tr("preferences.system")),
                    SelectItem::new("zh-CN", "简体中文"),
                    SelectItem::new("en-US", "English (US)"),
                ]
            },
            move || {
                match ui.preferences().language {
                    Language::System => "system",
                    Language::ZhCn => "zh-CN",
                    Language::EnUs => "en-US",
                }
                .into()
            },
            move || {
                match ui.preferences().language {
                    Language::System => ui.tr("preferences.system"),
                    Language::ZhCn => "简体中文",
                    Language::EnUs => "English (US)",
                }
                .into()
            },
            move |value| {
                ui.update(|preferences| {
                    preferences.language = match value.as_str() {
                        "zh-CN" => Language::ZhCn,
                        "en-US" => Language::EnUs,
                        _ => Language::System,
                    }
                })
            },
        ))
        .child(when(|| os() != Os::Android, move || backend_choice(ui)))
        .child(hint(move || match ui.state() {
            SaveState::Loading => ui.tr("preferences.loading").to_owned(),
            SaveState::Ready => ui.tr("preferences.saved").to_owned(),
            SaveState::Saving => ui.tr("preferences.saving").to_owned(),
            SaveState::LoadFailed(error) => {
                format!("{}: {error}", ui.tr("preferences.load_failed"))
            }
            SaveState::SaveFailed(error) => {
                format!("{}: {error}", ui.tr("preferences.save_failed"))
            }
        }))
        .child(when(
            move || matches!(ui.state(), SaveState::LoadFailed(_)),
            move || {
                action(
                    move || ui.tr("preferences.retry"),
                    false,
                    || false,
                    move || ui.retry_load(),
                )
            },
        ))
        .child(preview(ui))
}

fn backend_choice(ui: Ui) -> impl IntoElement {
    view()
        .gap_2()
        .child(choice(
            ui,
            "preferences.default_backend",
            move || {
                vec![
                    SelectItem::new("native", ui.tr("preferences.backend_native")),
                    SelectItem::new("hc", ui.tr("preferences.backend_hc")),
                    SelectItem::new("herdr", ui.tr("preferences.backend_herdr")),
                ]
            },
            move || {
                match ui.preferences().default_backend {
                    SessionBackend::Native => "native",
                    SessionBackend::Hc => "hc",
                    SessionBackend::Herdr => "herdr",
                }
                .into()
            },
            move || {
                ui.tr(match ui.preferences().default_backend {
                    SessionBackend::Native => "preferences.backend_native",
                    SessionBackend::Hc => "preferences.backend_hc",
                    SessionBackend::Herdr => "preferences.backend_herdr",
                })
                .into()
            },
            move |value| {
                let backend = match value.as_str() {
                    "native" => SessionBackend::Native,
                    "hc" => SessionBackend::Hc,
                    "herdr" => SessionBackend::Herdr,
                    _ => return,
                };
                ui.update(|preferences| preferences.default_backend = backend);
            },
        ))
        .child(hint(move || ui.tr("preferences.default_backend_hint")))
}

fn choice(
    ui: Ui,
    label: &'static str,
    items: impl Fn() -> Vec<SelectItem> + 'static,
    value: impl Fn() -> String + 'static,
    title: impl Fn() -> String + 'static,
    change: impl Fn(String) + 'static,
) -> impl IntoElement {
    view()
        .gap_2()
        .child(text(move || ui.tr(label)).text_sm())
        .child(
            select()
                .items(items)
                .value(move || Some(SharedString::from(value())))
                .disabled(move || !ui.editable())
                .searchable(label == "preferences.theme")
                .search_placeholder(move || Some(SharedString::from(ui.tr("preferences.search"))))
                .search_accessibility_label(move || {
                    Some(SharedString::from(ui.tr("preferences.search")))
                })
                .accessibility_label(move || Some(SharedString::from(ui.tr(label))))
                .on_change(move |next: &Option<SharedString>| {
                    if let Some(next) = next {
                        change(next.to_string());
                    }
                })
                .popup_style(|style| {
                    style
                        .p_1()
                        .rounded(move || theme().radius)
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().card)
                })
                .search_style(|style| {
                    style
                        .min_h(px(48.0))
                        .px_3()
                        .py_2()
                        .bg(move || theme().card)
                        .text_color(move || theme().foreground)
                })
                .item_style(|style| {
                    style
                        .min_h(move || px(if os() == Os::Android { 48.0 } else { 32.0 }))
                        .text_color(move || theme().foreground)
                        .rounded(move || theme().radius)
                })
                .item_class("focus:bg-accent selected:bg-secondary disabled:opacity-50")
                .render_item(|item: &SelectEntry| {
                    view()
                        .px_3()
                        .py_2()
                        .child(text(item.title.clone()).text_sm())
                })
                .empty(hint(move || ui.tr("preferences.empty")))
                .trigger(
                    view()
                        .w_full()
                        .min_h(move || px(if os() == Os::Android { 48.0 } else { 32.0 }))
                        .px_3()
                        .py_2()
                        .justify_center()
                        .rounded(move || theme().radius)
                        .border_1()
                        .border_color(move || theme().input)
                        .bg(move || theme().card)
                        .text_color(move || theme().foreground)
                        .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
                        .active(|style| style.opacity(0.75))
                        .disabled_style(|style| style.opacity(0.45))
                        .child(text(title).text_sm()),
                ),
        )
}

fn preview(ui: Ui) -> impl IntoElement {
    view()
        .gap_2()
        .child(text(move || ui.tr("preferences.preview")).text_sm())
        .child(
            view()
                .p_4()
                .gap_2()
                .rounded(move || theme().radius)
                .bg(move || crate::preferences::terminal::color(ui.theme().background))
                .text_color(move || crate::preferences::terminal::color(ui.theme().foreground))
                .child(
                    text("neko@host:~$ printf 'Hello, 世界'")
                        .font_family("monospace")
                        .text_size(move || px(ui.preferences().font_size as f32)),
                )
                .child(
                    view().flex_row().flex_wrap().gap_2().children(
                        (0..16)
                            .map(move |index| {
                                text(format!("{index:02}"))
                                    .font_family("monospace")
                                    .text_size(move || px(ui.preferences().font_size as f32))
                                    .text_color(move || {
                                        crate::preferences::terminal::color(
                                            ui.theme().palette[index],
                                        )
                                    })
                            })
                            .collect::<Vec<_>>(),
                    ),
                ),
        )
}
