use crate::preferences::tr;
use nsh_base::{button, input, theme};
use nsh_core::{platform::Os, prelude::*};

#[path = "icons.rs"]
pub mod icons;

pub fn control_size() -> Points {
    px(if os() == Os::Android { 48.0 } else { 32.0 })
}

pub fn tooltip(label: impl IntoReactive<SharedString>) -> impl nsh_core::element::IntoTooltip {
    let label = label.into_reactive();
    nsh_base::tooltip()
        .px_2()
        .py_1()
        .rounded(move || theme().radius)
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .child(
            text(move || tr(label.get().as_str()))
                .text_sm()
                .text_color(move || theme().popover_foreground),
        )
}

pub fn icon_action(
    icon: icons::Icon,
    label: impl IntoReactive<SharedString>,
    selected: impl Fn() -> bool + 'static,
    disabled: impl Fn() -> bool + 'static,
    run: impl Fn() + 'static,
) -> impl IntoElement {
    let label = label.into_reactive();
    let label = computed(move || tr(label.get().as_str()));
    view()
        .flex_shrink(0.0)
        .tooltip(tooltip(move || label.get()))
        .child(
            button()
                .accessibility_label(move || label.get())
                .selected(selected)
                .disabled(disabled)
                .on_press(run)
                .w(control_size)
                .h(control_size)
                .flex_shrink(0.0)
                .items_center()
                .justify_center()
                .rounded(move || theme().radius)
                .bg(move || theme().background.opacity(0.0))
                .selected_style(|style| style.bg(move || theme().accent))
                .hover(|style| style.bg(move || theme().secondary))
                .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
                .active(|style| style.opacity(0.75))
                .disabled_style(|style| style.opacity(0.45))
                .child(icons::render(icon)),
        )
}

/// A popup owns this button's press behavior; keep one shared icon appearance.
pub fn icon_trigger(icon: icons::Icon, label: &'static str) -> impl IntoElement {
    button()
        .accessibility_label(move || tr(label))
        .w(control_size)
        .h(control_size)
        .flex_shrink(0.0)
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(move || theme().background.opacity(0.0))
        .hover(|style| style.bg(move || theme().secondary))
        .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
        .active(|style| style.opacity(0.75))
        .child(icons::render(icon))
}

pub fn session_row(
    icon: icons::Icon,
    title: impl IntoReactive<SharedString>,
    selected: impl Fn() -> bool + 'static,
    disabled: impl Fn() -> bool + 'static,
    run: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .selected(selected)
        .disabled(disabled)
        .on_press(run)
        .min_h(control_size)
        .px_2()
        .gap_2()
        .flex_row()
        .items_center()
        .rounded(move || theme().radius)
        .bg(move || theme().background.opacity(0.0))
        .selected_style(|style| style.bg(move || theme().accent))
        .hover(|style| style.bg(move || theme().secondary))
        .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
        .active(|style| style.opacity(0.75))
        .disabled_style(|style| style.opacity(0.45))
        .child(icons::render(icon))
        .child(text(title).text_sm().flex_1().min_w(px(0.0)))
}

pub fn action(
    label: impl IntoReactive<SharedString>,
    primary: bool,
    disabled: impl Fn() -> bool + 'static,
    run: impl Fn() + 'static,
) -> impl IntoElement {
    let label = label.into_reactive();
    button()
        .disabled(disabled)
        .on_press(run)
        .min_h(move || px(if os() == Os::Android { 48.0 } else { 32.0 }))
        .px_4()
        .py_2()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(move || {
            if primary {
                theme().primary
            } else {
                theme().secondary
            }
        })
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .active(|s| s.opacity(0.75))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(move || tr(label.get().as_str()))
                .text_sm()
                .text_color(move || {
                    if primary {
                        theme().primary_foreground
                    } else {
                        theme().secondary_foreground
                    }
                }),
        )
}
pub fn field(
    value: Signal<String>,
    label: &'static str,
    placeholder: &'static str,
) -> impl IntoElement {
    field_with_edit(value, label, placeholder, || {})
}

pub fn field_with_edit(
    value: Signal<String>,
    label: &'static str,
    placeholder: &'static str,
    on_edit: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .gap_1()
        .child(text(move || tr(label)).text_sm())
        .child(
            input()
                .label(move || tr(label))
                .value(move || value.get())
                .on_change(move |v: &str| {
                    value.set(v.into());
                    on_edit();
                })
                .placeholder(move || tr(placeholder).to_string())
                .placeholder_color(move || color(theme().muted_foreground))
                .caret_color(move || color(theme().primary))
                .selection_color(move || color(theme().accent))
                .min_h(move || px(if os() == Os::Android { 48.0 } else { 32.0 }))
                .px_3()
                .py_2()
                .border_1()
                .border_color(move || theme().input)
                .rounded(move || theme().radius)
                .bg(move || theme().card)
                .text_color(move || theme().foreground)
                .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .w_full(),
        )
}
pub fn hint(value: impl IntoReactive<SharedString>) -> impl IntoElement {
    let value = value.into_reactive();
    text(move || tr(value.get().as_str()))
        .text_sm()
        .text_color(move || theme().muted_foreground)
}
pub fn heading(value: &'static str) -> impl IntoElement {
    text(move || tr(value))
        .text_base()
        .font_weight(FontWeight::SEMIBOLD)
        .role(AccessibilityRole::Header)
}

pub fn tab(
    label: impl IntoReactive<SharedString>,
    selected: impl Fn() -> bool + 'static,
    run: impl Fn() + 'static,
) -> impl IntoElement {
    let label = label.into_reactive();
    button()
        .selected(selected)
        .on_press(run)
        .min_h(move || px(if os() == Os::Android { 48.0 } else { 32.0 }))
        .px_4()
        .py_2()
        .rounded(move || theme().radius)
        .bg(move || theme().secondary)
        .selected_style(|style| style.bg(move || theme().accent))
        .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
        .hover(|style| style.opacity(0.9))
        .active(|style| style.opacity(0.75))
        .child(
            text(move || tr(label.get().as_str()))
                .text_sm()
                .text_color(move || theme().secondary_foreground),
        )
}

/// A list row, rather than a full-width form action. Shared by devices and sessions.
pub fn navigation_row(
    title: impl IntoReactive<SharedString>,
    detail: impl IntoReactive<SharedString>,
    selected: impl Fn() -> bool + 'static,
    run: impl Fn() + 'static,
) -> impl IntoElement {
    navigation_row_disabled(title, detail, selected, || false, run)
}

pub fn navigation_row_disabled(
    title: impl IntoReactive<SharedString>,
    detail: impl IntoReactive<SharedString>,
    selected: impl Fn() -> bool + 'static,
    disabled: impl Fn() -> bool + 'static,
    run: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .selected(selected)
        .disabled(disabled)
        .disabled_style(|s| s.opacity(0.55))
        .on_press(run)
        .min_h(px(56.0))
        .px_3()
        .py_2()
        .gap_1()
        .rounded(move || theme().radius)
        .bg(move || theme().background.opacity(0.0))
        .selected_style(|s| s.bg(move || theme().accent))
        .hover(|s| s.bg(move || theme().secondary))
        .active(|s| s.opacity(0.75))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(text(title).text_sm().font_weight(FontWeight::MEDIUM))
        .child(
            text(detail)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

#[cfg(test)]
mod translation_tests {
    use super::*;
    use crate::preferences::{Preferences, Ui, catalog::Catalog};
    use nsh_core::{
        element::Root,
        platform::{Environment, Locale, Os},
    };
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;

    #[component]
    fn Fixture() -> impl IntoElement {
        Ui::from_loaded(Preferences::defaults(
            Catalog::shared().expect("test catalog"),
        ))
        .expect("reactive preference context");
        view()
            .child(heading("设备与会话"))
            .child(action("新建终端", false, || false, || {}))
            .child(icon_action(
                icons::Icon::Files,
                "文件",
                || false,
                || false,
                || {},
            ))
            .child(field(signal(String::new()), "终端", "Relay 地址"))
            .child(tab("终端", || true, || {}))
            .child(session_row(
                icons::Icon::Terminal,
                "本机",
                || false,
                || false,
                || {},
            ))
            .child(icons::activity(Some("working")))
            .child(
                nsh_base::button()
                    .on_press(|| set_locale(Some(Locale::EN_US)))
                    .child(text("English fixture")),
            )
            .child(
                nsh_base::button()
                    .on_press(|| set_locale(Locale::parse("zh-CN")))
                    .child(text("Chinese fixture")),
            )
    }

    #[test]
    fn mounted_controls_translate_both_ways_and_keep_tooltips_and_session_names()
    -> Result<(), String> {
        let mut app = Headless::start(EngineConfig {
            environment: Environment {
                os: Os::Linux,
                locale: Locale::parse("zh-CN").expect("Chinese locale"),
                ..Environment::default()
            },
            ..EngineConfig::new(Root::of::<Fixture>())
        })
        .map_err(|error| error.to_string())?;
        app.wait_for("设备与会话")?;
        assert!(app.screen().press_target("文件")?.is_some());
        app.press("English fixture")?;
        app.wait_for("Devices and sessions")?;
        app.wait_for("New terminal")?;
        app.wait_for("Working")?;
        app.wait_for("Terminal")?;
        assert!(app.screen().press_target("Files")?.is_some());
        assert!(app.shows("本机"));
        assert!(!app.shows("设备与会话"));
        app.hover("Files")?;
        app.wait_for("Files")?;
        app.press("Chinese fixture")?;
        app.wait_for("设备与会话")?;
        app.wait_for("新建终端")?;
        app.wait_for("工作中")?;
        assert!(app.screen().press_target("文件")?.is_some());
        assert!(!app.shows("Devices and sessions"));
        app.hover("文件")?;
        app.wait_for("文件")?;
        Ok(())
    }
}
