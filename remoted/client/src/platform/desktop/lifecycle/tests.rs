use super::*;
use nsh_core::{
    Command, EventKind, EventPayload, PayloadValue, ROOT_ID,
    desktop::{DesktopCommand, MenuItem, Tray, WindowRequest},
    element::Root,
    platform::{Environment, Locale, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[component]
fn Fixture() -> impl IntoElement {
    let settings = signal(false);
    let mut lifecycle = Lifecycle::new();
    lifecycle.settings = Some(settings);
    let policy = signal(CloseBehavior::Ask);
    let available = signal(true);
    let saved = signal(0_u32);
    let acknowledgement = signal(None::<bool>);
    let reject_immediately = signal(false);
    let pending = signal(false);
    let pulses = signal(0_u32);
    let labels = Labels::new(Language::EnUs);
    lifecycle.install_tray(labels);
    let mut attributes = WindowAttributes::default();
    attributes.close_on_press = Some(false);
    desktop::main_window().set(attributes);
    let close = move || {
        lifecycle.close(
            policy.get_untracked(),
            labels,
            move |choice| async move {
                pending.set(true);
                while !reject_immediately.get_untracked()
                    && acknowledgement.get_untracked().is_none()
                {
                    timer(Duration::from_millis(10)).await;
                }
                pending.set(false);
                if !reject_immediately.get_untracked()
                    && acknowledgement.get_untracked() == Some(true)
                {
                    policy.set(choice);
                    saved.update(|count| *count += 1);
                    Ok(())
                } else {
                    Err("Test storage refused the write".to_owned())
                }
            },
            move || std::future::ready(available.get_untracked()),
        );
    };
    desktop::main_window().on_close_press(move |_| close());
    spawn(async move {
        loop {
            timer(Duration::from_millis(40)).await;
            pulses.update(|count| *count += 1);
        }
    });
    view()
        .child(lifecycle.render())
        .child(text(move || format!("close busy={}", lifecycle.busy.get())))
        .child(
            nsh_base::button()
                .on_press(move || settings.set(true))
                .child(text("Open settings")),
        )
        .child(
            desktop::window()
                .id("settings")
                .parent("main")
                .title("Settings fixture")
                .open(settings)
                .on_open_change(move |event| settings.set(event.open))
                .content(move || {
                    view().child(text("Settings fixture")).child(
                        nsh_base::button()
                            .on_press(close)
                            .child(text("Close main from settings")),
                    )
                }),
        )
        .child(text("Mounted terminal"))
        .child(text(move || {
            format!(
                "policy={:?};saved={};pending={}",
                policy.get(),
                saved.get(),
                pending.get()
            )
        }))
        .child(text(move || format!("terminal work {}", pulses.get())))
        .child(
            nsh_base::button()
                .on_press(move || acknowledgement.set(Some(true)))
                .child(text("Acknowledge save")),
        )
        .child(
            nsh_base::button()
                .on_press(move || acknowledgement.set(Some(false)))
                .child(text("Reject save")),
        )
        .child(
            nsh_base::button()
                .on_press(move || reject_immediately.set(true))
                .child(text("Reject saves immediately")),
        )
        .child(
            nsh_base::button()
                .on_press(move || available.set(false))
                .child(text("Remove tray host")),
        )
        .child(
            nsh_base::button()
                .on_press(move || policy.set(CloseBehavior::Quit))
                .child(text("Saved Quit")),
        )
        .child(
            nsh_base::button()
                .on_press(move || policy.set(CloseBehavior::Tray))
                .child(text("Saved Tray")),
        )
}

fn app() -> Headless {
    let mut app = Headless::start(EngineConfig {
        clock: nsh_core::time::Clock::manual(std::time::SystemTime::UNIX_EPOCH),
        environment: Environment {
            os: Os::Linux,
            locale: Locale::EN_US,
            ..Default::default()
        },
        ..EngineConfig::new(Root::of::<Fixture>())
    })
    .expect("start task-owned lifecycle fixture");
    app.answer_dialogs_myself();
    app.wait_for("Mounted terminal")
        .expect("fixture is mounted");
    app
}

#[component]
fn SavedQuitFixture() -> impl IntoElement {
    let catalog = crate::preferences::catalog::Catalog::shared().expect("bundled preferences");
    let mut preferences = crate::preferences::Preferences::defaults(catalog);
    preferences.close_behavior = CloseBehavior::Quit;
    let preferences = Ui::from_loaded(preferences).expect("loaded preferences");
    let lifecycle = start_desktop_lifecycle(preferences, signal(false));
    let close = use_context::<CloseRequest>().expect("production lifecycle provides close policy");
    view().child(lifecycle).child(
        nsh_base::button()
            .on_press(move || close(CloseReason::CloseButton))
            .child(text("Request close policy")),
    )
}

#[test]
fn production_close_context_and_native_close_share_the_saved_quit_policy() -> Result<(), String> {
    for trigger in ["enter", "space", "native"] {
        let mut app = Headless::start(EngineConfig {
            clock: nsh_core::time::Clock::manual(std::time::SystemTime::UNIX_EPOCH),
            environment: Environment {
                os: Os::Linux,
                locale: Locale::EN_US,
                ..Default::default()
            },
            ..EngineConfig::new(Root::of::<SavedQuitFixture>())
        })
        .map_err(|error| error.to_string())?;
        app.wait_for("Request close policy")?;
        if trigger == "native" {
            app.press_window_close("main")?;
        } else {
            app.press_key("tab")?;
            app.press_key(trigger)?;
        }
        assert_eq!(
            app.screen()
                .commands()
                .iter()
                .filter(|(_, command)| matches!(
                    command,
                    Command::Desktop(command) if matches!(**command, DesktopCommand::Quit)
                ))
                .count(),
            1,
            "{trigger} must request one policy-controlled quit"
        );
        assert!(app.screen().dialogs().is_empty());
        assert!(!app.shows("Close neoko-terminal?"));
    }
    Ok(())
}

fn quit_requested(app: &Headless) -> bool {
    app.screen().commands().iter().any(|(_, command)| {
        matches!(command, Command::Desktop(command) if matches!(**command, DesktopCommand::Quit))
    })
}

fn tray(app: &Headless) -> &Tray {
    app.screen()
        .commands()
        .iter()
        .rev()
        .find_map(|(_, command)| match command {
            Command::Desktop(command) => match &**command {
                DesktopCommand::SetTray(Some(tray)) => Some(tray),
                _ => None,
            },
            _ => None,
        })
        .expect("the fixture installs a genuine NSH tray command")
}

fn select(app: &mut Headless, id: u32) -> Result<(), String> {
    app.engine().control().dispatch_event(
        ROOT_ID,
        EventKind::Menu,
        EventPayload::Map(vec![("id", PayloadValue::Number(f64::from(id)))]),
    );
    app.run_for(Duration::from_millis(120))
}

fn tray_action(app: &mut Headless, label: &str) -> Result<(), String> {
    let id = tray(app)
        .menu
        .iter()
        .find_map(|item| match item {
            MenuItem::Action(action) if action.label == label => Some(action.id),
            _ => None,
        })
        .expect("tray action exists");
    select(app, id)
}

#[test]
fn first_close_has_one_semantic_native_decision_and_cancel_does_not_save() -> Result<(), String> {
    let mut app = app();
    app.press_window_close("main")?;
    app.wait_for("Close neoko-terminal?")?;
    assert!(app.shows("Cancel"));
    assert!(app.shows("Minimize to tray"));
    assert!(app.shows("Quit"));
    assert_eq!(app.screen().count("role:alertdialog")?, 1);
    app.press_window_close("main")?;
    assert_eq!(
        app.screen()
            .texts()
            .iter()
            .filter(|text| **text == "Close neoko-terminal?")
            .count(),
        1,
        "repeated close does not stack prompts"
    );
    app.press("Cancel")?;
    app.wait_for("policy=Ask;saved=0;pending=false")?;
    assert_eq!(app.screen().count("role:alertdialog")?, 0);
    assert!(app.window_state("main")?.unwrap().visible);
    assert!(!quit_requested(&app));
    app.press_window_close("main")?;
    app.press_key("escape")?;
    assert!(app.window_state("main")?.unwrap().visible);
    app.wait_for("policy=Ask;saved=0;pending=false")?;
    Ok(())
}

#[test]
fn closing_settings_does_not_start_or_block_the_main_close_policy() -> Result<(), String> {
    let mut app = app();
    app.press("Open settings")?;
    app.wait_for("Settings fixture")?;
    app.press_window_close("settings")?;
    assert!(app.window_state("settings")?.is_none());
    app.wait_for("close busy=false")?;
    assert!(!app.shows("Close neoko-terminal?"));
    assert!(!quit_requested(&app));
    app.press_window_close("main")?;
    app.wait_for("Close neoko-terminal?")?;
    app.press("Cancel")?;
    app.wait_for("close busy=false")?;
    Ok(())
}

#[test]
fn main_prompt_survives_settings_owner_disposal_and_cancel_allows_another_close()
-> Result<(), String> {
    let mut app = app();
    app.press("Open settings")?;
    app.wait_for("Settings fixture")?;
    app.press("Close main from settings")?;
    app.wait_for("Close neoko-terminal?")?;
    assert!(app.window_state("settings")?.is_none());
    app.wait_for("close busy=true")?;
    app.press("Cancel")?;
    app.wait_for("close busy=false")?;
    assert!(app.window_state("main")?.unwrap().visible);
    app.press("Open settings")?;
    app.wait_for("Settings fixture")?;
    app.press_window_close("main")?;
    app.wait_for("Close neoko-terminal?")?;
    assert!(app.window_state("settings")?.is_none());
    app.press_key("escape")?;
    app.wait_for("close busy=false")?;
    app.wait_for("policy=Ask;saved=0;pending=false")?;
    assert!(!app.shows("Close neoko-terminal?"));
    assert!(!quit_requested(&app));
    Ok(())
}

#[test]
fn immediate_save_failure_after_settings_disposal_remains_cancelable() -> Result<(), String> {
    let mut app = app();
    app.press("Reject saves immediately")?;
    app.press("Open settings")?;
    app.wait_for("Settings fixture")?;
    app.press("Close main from settings")?;
    app.wait_for("Close neoko-terminal?")?;
    app.press("Quit")?;
    app.wait_for("Test storage refused the write")?;
    assert!(app.window_state("settings")?.is_none());
    assert!(app.window_state("main")?.unwrap().visible);
    assert!(!quit_requested(&app));
    app.press("OK")?;
    app.wait_for("close busy=false")?;
    app.press_window_close("main")?;
    app.wait_for("Close neoko-terminal?")?;
    app.press("Cancel")?;
    app.wait_for("close busy=false")?;
    app.wait_for("policy=Ask;saved=0;pending=false")?;
    Ok(())
}

#[test]
fn fallback_from_settings_owner_clears_busy_on_escape() -> Result<(), String> {
    let mut app = app();
    app.press("Saved Tray")?;
    app.press("Remove tray host")?;
    app.press("Open settings")?;
    app.wait_for("Settings fixture")?;
    app.press("Close main from settings")?;
    app.wait_for("System tray unavailable")?;
    assert!(app.window_state("settings")?.is_none());
    app.press_key("escape")?;
    app.wait_for("close busy=false")?;
    assert!(!app.window_state("main")?.unwrap().minimized);
    app.press_window_close("main")?;
    app.wait_for("System tray unavailable")?;
    app.press("Minimize to taskbar")?;
    assert!(app.window_state("main")?.unwrap().minimized);
    assert!(!quit_requested(&app));
    Ok(())
}

#[test]
fn tray_choice_waits_for_ack_then_hides_and_native_tray_click_restores() -> Result<(), String> {
    let mut app = app();
    app.press_window_close("main")?;
    app.press("Minimize to tray")?;
    app.wait_for("policy=Ask;saved=0;pending=true")?;
    assert!(
        app.window_state("main")?.unwrap().visible,
        "pending writes must not hide"
    );
    let before = app.screen().texts().join("\n");
    app.press("Acknowledge save")?;
    app.run_for(Duration::from_millis(40))?;
    assert!(!app.window_state("main")?.unwrap().visible);
    assert!(
        app.screen().dialogs().is_empty(),
        "the main-window decision must not create a platform alert"
    );
    assert!(!app.shows("Close neoko-terminal?"));
    app.run_for(Duration::from_millis(140))?;
    let click = tray(&app).click;
    assert_ne!(click, 0);
    select(&mut app, click)?;
    app.wait_for("policy=Tray;saved=1;pending=false")?;
    assert_eq!(
        app.screen().count("role:alertdialog")?,
        0,
        "the acknowledged choice must not leave another confirmation"
    );
    assert_ne!(
        app.screen().texts().join("\n"),
        before,
        "terminal work continues while hidden"
    );
    assert!(app.shows("Mounted terminal"));
    assert!(app.window_state("main")?.unwrap().visible);
    assert!(!app.window_state("main")?.unwrap().minimized);
    app.press_window_close("main")?;
    assert!(
        !app.window_state("main")?.unwrap().visible,
        "saved Tray is applied without prompting"
    );
    tray_action(&mut app, "Show terminal")?;
    assert!(app.window_state("main")?.unwrap().visible);
    assert!(!quit_requested(&app));
    Ok(())
}

#[test]
fn failed_remember_never_quits_or_applies_the_policy() -> Result<(), String> {
    let mut app = app();
    app.press_window_close("main")?;
    app.press("Quit")?;
    app.wait_for("policy=Ask;saved=0;pending=true")?;
    assert!(!quit_requested(&app));
    app.press("Reject save")?;
    app.run_for(Duration::from_millis(40))?;
    app.wait_for("Test storage refused the write")?;
    assert!(app.window_state("main")?.unwrap().visible);
    assert!(!quit_requested(&app));
    app.press("OK")?;
    app.wait_for("policy=Ask;saved=0;pending=false")?;
    Ok(())
}

#[test]
fn remembered_quit_and_tray_quit_request_actual_app_shutdown() -> Result<(), String> {
    let mut app = app();
    app.press("Saved Quit")?;
    app.press_window_close("main")?;
    assert!(quit_requested(&app));
    assert!(app.screen().dialogs().is_empty());
    assert!(!app.screen().commands().iter().any(|(_, command)| {
        matches!(command, Command::Desktop(command) if matches!(**command, DesktopCommand::Request(WindowRequest::Close)))
    }), "Quit must end the application, not merely close its last window");
    let mut app = self::app();
    app.press("Saved Tray")?;
    app.press_window_close("main")?;
    assert!(!app.window_state("main")?.unwrap().visible);
    tray_action(&mut app, "Quit")?;
    assert!(quit_requested(&app));
    Ok(())
}

#[test]
fn unconfirmed_tray_falls_back_to_taskbar_without_chaining_first_close_prompts()
-> Result<(), String> {
    let mut app = app();
    app.press("Remove tray host")?;
    app.press_window_close("main")?;
    assert!(app.screen().texts().join(" ").contains("taskbar"));
    app.press("Minimize to tray")?;
    app.press("Acknowledge save")?;
    app.run_for(Duration::from_millis(40))?;
    let window = app.window_state("main")?.unwrap();
    assert!(window.minimized, "taskbar restore remains accessible");
    assert!(!app.screen().commands().iter().any(|(_, command)| {
        matches!(command, Command::Desktop(command) if matches!(**command, DesktopCommand::Request(WindowRequest::Hide)))
    }), "fallback must minimize rather than hide the window");
    assert!(!app.shows("Close neoko-terminal?"));
    assert!(!quit_requested(&app));
    tray_action(&mut app, "Show terminal")?;
    let window = app.window_state("main")?.unwrap();
    assert!(window.visible && !window.minimized);
    Ok(())
}

#[test]
fn restoration_invalidates_an_inflight_close() -> Result<(), String> {
    let mut app = app();
    app.press_window_close("main")?;
    app.press("Minimize to tray")?;
    app.wait_for("policy=Ask;saved=0;pending=true")?;
    tray_action(&mut app, "Show terminal")?;
    app.press("Acknowledge save")?;
    app.run_for(Duration::from_millis(40))?;
    app.wait_for("policy=Tray;saved=1;pending=false")?;
    assert!(
        app.window_state("main")?.unwrap().visible,
        "late close must not hide a restored window"
    );
    assert!(!quit_requested(&app));
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn losing_the_linux_tray_host_restores_the_hidden_window() -> Result<(), String> {
    let mut app = app();
    let remove = app.screen().press_target("Remove tray host")?.unwrap();
    app.press("Saved Tray")?;
    app.press_window_close("main")?;
    assert!(!app.window_state("main")?.unwrap().visible);
    app.engine()
        .control()
        .dispatch_event(remove, EventKind::Press, EventPayload::Map(Vec::new()));
    app.run_for(Duration::from_millis(2200))?;
    assert!(app.window_state("main")?.unwrap().visible);
    assert!(app.screen().texts().join(" ").contains("restored"));
    assert!(!quit_requested(&app));
    Ok(())
}

#[cfg(target_os = "linux")]
static NATIVE_PHASE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "linux")]
#[component]
fn NativeFixture() -> impl IntoElement {
    let lifecycle = Lifecycle::new();
    let labels = Labels::new(Language::EnUs);
    lifecycle.install_tray(labels);
    let pulses = signal(0_u32);
    spawn(async move {
        loop {
            timer(Duration::from_millis(40)).await;
            pulses.update(|count| *count += 1);
        }
    });
    spawn(async move {
        timer(Duration::from_millis(400)).await;
        for phase in 1..=3 {
            lifecycle.close(
                CloseBehavior::Tray,
                labels,
                |_| std::future::ready(Ok(())),
                tray_available,
            );
            loop {
                timer(Duration::from_millis(30)).await;
                if desktop::main_window()
                    .state()
                    .is_some_and(|window| !window.visible)
                {
                    break;
                }
            }
            NATIVE_PHASE.store(phase, std::sync::atomic::Ordering::SeqCst);
            let runtime = std::env::var_os("XDG_RUNTIME_DIR").expect("isolated runtime directory");
            std::fs::write(
                std::path::Path::new(&runtime).join("native-stage"),
                phase.to_string(),
            )
            .expect("publish native test stage");
            if phase < 3 {
                loop {
                    timer(Duration::from_millis(30)).await;
                    if desktop::main_window()
                        .state()
                        .is_some_and(|window| window.visible)
                    {
                        break;
                    }
                }
            }
        }
    });
    spawn(async {
        timer(Duration::from_secs(15)).await;
        // Keep a failed native test bounded; a successful tray Quit ends sooner.
        NATIVE_PHASE.store(0, std::sync::atomic::Ordering::SeqCst);
        desktop::quit();
    });
    view().child(lifecycle.render()).child(text(move || {
        format!("Task-owned terminal work {}", pulses.get())
    }))
}

/// Run only under a task-owned headless compositor and private D-Bus watcher.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires an isolated headless compositor and StatusNotifier test host"]
fn native_tray_primary_restore_menu_restore_and_quit() {
    let runtime = std::env::var("XDG_RUNTIME_DIR").expect("isolated runtime directory");
    assert!(
        runtime.starts_with("/tmp/neko-lifecycle-native-"),
        "never run this fixture on the user's desktop"
    );
    assert_eq!(std::env::var("WLR_BACKENDS").as_deref(), Ok("headless"));
    let exit = nsh_platform_linux::run_app(
        nsh_platform_linux::AppSource::Rust(Root::of::<NativeFixture>()),
        nsh_platform_linux::AppOptions {
            title: "neoko lifecycle test".into(),
            app_id: "community.lazycat.neko.lifecycle".into(),
            size: (640, 400),
            ..Default::default()
        },
    );
    assert_eq!(
        std::process::Termination::report(exit),
        std::process::ExitCode::SUCCESS
    );
    assert_eq!(
        NATIVE_PHASE.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "native tray actions must restore both ways before Quit ends the actual app"
    );
}
