//! Native window actions: the close control follows the same policy as the system.
use super::controls::{
    icons::{self, Icon},
    tooltip,
};
use crate::preferences::tr;
use nsh_base::theme;
use nsh_core::{
    desktop::{CloseReason, WindowControl, WindowMode},
    element::desktop,
    prelude::*,
};

// Matches the app-owned callback provided by start_desktop_lifecycle.
type CloseRequest = std::rc::Rc<dyn Fn(CloseReason)>;

pub(super) fn render() -> impl IntoElement {
    when(
        || os().is_desktop(),
        || {
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(control(
                    WindowControl::Minimize,
                    "最小化窗口",
                    Icon::Minimize,
                ))
                .child(
                    view()
                        .tooltip(tooltip(move || {
                            if maximized() {
                                tr("还原窗口")
                            } else {
                                tr("最大化窗口")
                            }
                        }))
                        .child(
                            view()
                                .role(AccessibilityRole::Button)
                                .focusable(true)
                                .label(move || {
                                    if maximized() {
                                        tr("还原窗口")
                                    } else {
                                        tr("最大化窗口")
                                    }
                                })
                                .window_drag(false)
                                .window_control(WindowControl::Maximize)
                                .on_press(keyboard_action(WindowControl::Maximize))
                                .w(px(32.0))
                                .h(px(32.0))
                                .items_center()
                                .justify_center()
                                .rounded(move || theme().radius)
                                .bg(move || theme().background.opacity(0.0))
                                .hover(|s| s.bg(move || theme().secondary))
                                .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                                .child(
                                    when(maximized, || icons::render(Icon::Restore))
                                        .otherwise(|| icons::render(Icon::Maximize)),
                                ),
                        ),
                )
                .child(control(WindowControl::Close, "关闭窗口", Icon::Close))
        },
    )
}
fn maximized() -> bool {
    desktop::main_window()
        .state()
        .is_some_and(|window| window.mode == WindowMode::Maximized)
}
fn control(action: WindowControl, label: &'static str, icon: Icon) -> impl IntoElement {
    view().tooltip(tooltip(move || tr(label))).child(
        view()
            .role(AccessibilityRole::Button)
            .focusable(true)
            .label(move || tr(label))
            .window_drag(false)
            .window_control(action)
            .on_press(keyboard_action(action))
            .w(px(32.0))
            .h(px(32.0))
            .items_center()
            .justify_center()
            .rounded(move || theme().radius)
            .bg(move || theme().background.opacity(0.0))
            .hover(|s| s.bg(move || theme().secondary))
            .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
            .child(icons::render(icon)),
    )
}

// Native pointer and AT activation already execute window_control. Only a
// non-pointer Press (the keyboard's empty payload) needs the app fallback.
fn keyboard_action(action: WindowControl) -> impl Fn(&PressEvent) {
    let close = use_context::<CloseRequest>();
    move |event| {
        if event.clicks != 0 {
            return;
        }
        let window = desktop::main_window();
        match action {
            WindowControl::Minimize => window.minimize(),
            WindowControl::Maximize => window.set_mode(if maximized() {
                WindowMode::Normal
            } else {
                WindowMode::Maximized
            }),
            WindowControl::Close => {
                if let Some(close) = &close {
                    close(CloseReason::CloseButton);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::{
        EventKind, MountItem,
        element::Root,
        platform::{Environment, Locale, Os},
    };
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;
    use std::sync::{Arc, Mutex};

    #[component]
    fn Fixture() -> impl IntoElement {
        let requests = signal(0_u32);
        provide_context::<CloseRequest>(std::rc::Rc::new(move |reason| {
            assert_eq!(reason, CloseReason::CloseButton);
            requests.update(|requests| *requests += 1);
        }));
        view()
            .child(text(move || format!("close requests={}", requests.get())))
            .child(render())
            .child(text("Window controls fixture"))
            .child(
                nsh_base::button()
                    .on_press(|| desktop::main_window().set_mode(WindowMode::Maximized))
                    .child(text("Maximize fixture")),
            )
            .child(
                nsh_base::button()
                    .on_press(|| desktop::main_window().set_mode(WindowMode::Normal))
                    .child(text("Restore fixture")),
            )
    }

    fn app(os: Os) -> Result<Headless, String> {
        Headless::start(EngineConfig {
            clock: nsh_core::time::Clock::manual(std::time::SystemTime::UNIX_EPOCH),
            environment: Environment {
                os,
                locale: Locale::EN_US,
                ..Default::default()
            },
            ..EngineConfig::new(Root::of::<Fixture>())
        })
        .map_err(|error| error.to_string())
    }

    #[test]
    fn named_focusable_controls_keep_native_actions_and_listen_for_keyboard_activation()
    -> Result<(), String> {
        let mut app = app(Os::Linux)?;
        let mounted = Arc::new(Mutex::new(Vec::new()));
        let observed = mounted.clone();
        app.observe(move |_, batch| {
            observed
                .lock()
                .expect("capture mounted native controls")
                .extend(batch.items.iter().cloned());
        });
        app.wait_for("Window controls fixture")?;
        let mounted = mounted.lock().expect("inspect mounted native controls");
        for (action, label) in [
            (WindowControl::Minimize, "Minimize window"),
            (WindowControl::Maximize, "Maximize window"),
            (WindowControl::Close, "Close window"),
        ] {
            let controls = mounted
                .iter()
                .filter_map(|item| match item {
                    MountItem::ViewProps { id, props } if props.window_control == Some(action) => {
                        Some((*id, props))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(controls.len(), 1, "one native control for {action:?}");
            let (id, props) = controls[0];
            assert_eq!(props.window_drag, Some(false));
            assert_eq!(props.focusable, Some(true));
            assert_eq!(props.accessibility.role, AccessibilityRole::Button);
            assert_eq!(props.accessibility.label.as_deref(), Some(label));
            assert!(
                mounted.iter().any(|item| matches!(
                    item,
                    MountItem::Events { id: node, events }
                        if *node == id && events.contains(EventKind::Press)
                )),
                "{action:?} needs a Press listener for native keyboard activation"
            );
        }
        Ok(())
    }

    #[test]
    fn maximize_and_restore_follow_native_state_and_keep_the_accessible_name_current()
    -> Result<(), String> {
        let mut app = app(Os::Linux)?;
        app.wait_for("label:Maximize window")?;
        assert!(!app.screen().matches("label:Restore window")?);
        app.press("Maximize fixture")?;
        app.wait_for("label:Restore window")?;
        assert!(!app.screen().matches("label:Maximize window")?);
        assert_eq!(
            app.window_state("main")?.unwrap().mode,
            WindowMode::Maximized
        );
        app.press("Restore fixture")?;
        assert_eq!(app.window_state("main")?.unwrap().mode, WindowMode::Normal);
        app.wait_for("label:Maximize window")?;
        assert!(!app.screen().matches("label:Restore window")?);
        Ok(())
    }

    #[test]
    fn enter_and_space_close_request_the_app_policy_once_without_disposing_the_window()
    -> Result<(), String> {
        for key in ["enter", "space"] {
            let mut app = app(Os::Linux)?;
            app.wait_for("close requests=0")?;
            for _ in 0..3 {
                app.press_key("tab")?;
            }
            app.press_key(key)?;
            app.wait_for("close requests=1")?;
            assert!(
                app.window_state("main")?
                    .is_some_and(|window| window.visible)
            );
            app.press_key(key)?;
            app.wait_for("close requests=2")?;
        }
        Ok(())
    }

    #[test]
    fn enter_and_space_toggle_maximization_and_minimize() -> Result<(), String> {
        for key in ["enter", "space"] {
            let mut app = app(Os::Linux)?;
            app.wait_for("Window controls fixture")?;
            app.press_key("tab")?;
            app.press_key("tab")?;
            app.press_key(key)?;
            assert_eq!(
                app.window_state("main")?.unwrap().mode,
                WindowMode::Maximized
            );
            app.wait_for("label:Restore window")?;
            app.press_key(key)?;
            assert_eq!(app.window_state("main")?.unwrap().mode, WindowMode::Normal);
            app.wait_for("label:Maximize window")?;
            app.press_key("shift+tab")?;
            app.press_key(key)?;
            assert!(app.window_state("main")?.unwrap().minimized);
        }
        Ok(())
    }

    #[test]
    fn pointer_presses_leave_all_actions_to_native_window_control() -> Result<(), String> {
        let mut app = app(Os::Linux)?;
        app.wait_for("Window controls fixture")?;
        for label in ["Maximize window", "Close window", "Minimize window"] {
            let target = app
                .screen()
                .press_target(&format!("label:{label}"))?
                .unwrap();
            for clicks in [1, 2] {
                app.engine().control().dispatch_event(
                    target,
                    EventKind::Press,
                    nsh_core::EventPayload::Press(PressEvent {
                        clicks,
                        ..Default::default()
                    }),
                );
                app.run_for(std::time::Duration::from_millis(20))?;
                let window = app.window_state("main")?.unwrap();
                assert_eq!(window.mode, WindowMode::Normal);
                assert!(!window.minimized);
                assert!(window.visible);
                assert!(app.shows("close requests=0"));
            }
        }
        Ok(())
    }

    #[test]
    fn mobile_does_not_mount_desktop_window_controls() -> Result<(), String> {
        let mut app = app(Os::Android)?;
        app.wait_for("Window controls fixture")?;
        for label in [
            "Minimize window",
            "Maximize window",
            "Restore window",
            "Close window",
        ] {
            assert!(!app.screen().matches(&format!("label:{label}"))?);
        }
        Ok(())
    }
}
