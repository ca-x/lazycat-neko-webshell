//! Standalone integration tests loaded by check.mjs, without modifying the host module tree.

use crate::preferences::{self, Appearance, Language, Preferences, Ui, catalog::Catalog};
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Environment, Root};
use nsh_testing::Headless;

#[component]
fn Fixture() -> impl IntoElement {
    let ui = Ui::start().expect("valid test catalog");
    let terminal = preferences::current_context();
    view()
        .child(text(move || format!("backend={:?}", ui.startup_backend())))
        .child(crate::ui::preferences::render(ui))
        .child(text(move || preferences::tr("终端")))
        .child(text(move || preferences::tr_format("主机 {}", &[&7])))
        .child(text(move || preferences::tr("已退出（3）")))
        .child(text(move || {
            format!(
                "font={}; appearance={:?}; language={:?}",
                ui.preferences().font_size,
                if dark() { "Dark" } else { "Light" },
                locale()
            )
        }))
        .child(
            nsh_base::button()
                .on_press(move || {
                    ui.update(|preferences| {
                        preferences.font_size = 20;
                        preferences.language = Language::EnUs;
                        preferences.appearance = Appearance::Dark;
                        preferences.theme_id = "light".into();
                    })
                })
                .child(text("Apply fixture")),
        )
        .child(
            nsh_base::button()
                .on_press(move || {
                    ui.update(|preferences| {
                        preferences.language = Language::System;
                        preferences.appearance = Appearance::System;
                    })
                })
                .child(text("Follow system")),
        )
        .child(text(move || format!("state={:?}", ui.state())))
        .child(text(move || format!("theme={}", ui.theme().id)))
        .child(
            text("Mounted terminal font")
                .font_family(crate::fonts::terminal_family)
                .text_size(move || px(terminal.font_size() as f32))
                .line_height(move || px(terminal.line_height())),
        )
        .child(text(move || {
            format!(
                "cursor={};blink={};auto_control={}",
                terminal.cursor_style().as_str(),
                terminal.cursor_blink(),
                ui.preferences().remote_auto_control,
            )
        }))
        .child(text(move || {
            format!("close={}", ui.preferences().close_behavior.as_str())
        }))
}

const FONT_PREVIEW: &str = "neko@host:~$ printf 'Hello, 世界'";

fn assert_font(app: &Headless, family: &str, size: f32) -> Result<(), String> {
    for selector in [FONT_PREVIEW, "Mounted terminal font"] {
        let style = app.screen().span_style(selector)?;
        assert_eq!(style.font_size.0, size, "font size for {selector}");
        assert_eq!(
            style.font_family.expect("terminal family").name().as_ref(),
            crate::fonts::resolve_terminal_family(family).as_ref(),
            "font family for {selector}",
        );
    }
    Ok(())
}

#[test]
fn terminal_preferences_custom_font_waits_for_save_and_restores_in_existing_bindings()
-> Result<(), String> {
    use crate::preferences::font;
    reset(None);
    let mut app = app();
    app.wait_for("state=Ready")?;
    assert_font(&app, font::HACK, 14.0)?;
    app.clear("label:已安装的字体名称")?;
    app.type_text("label:已安装的字体名称", "JetBrains Mono")?;
    assert_font(&app, font::HACK, 14.0)?;
    assert_eq!(crate::platform::STORE.lock().expect("test store").writes, 0);

    let gate = std::sync::Arc::new(tokio::sync::Notify::new());
    crate::platform::STORE
        .lock()
        .expect("test store")
        .write_gate = Some(gate.clone());
    app.press("保存字体")?;
    app.wait_for("state=Saving")?;
    assert_font(&app, font::HACK, 14.0)?;
    assert!(!app.screen().enabled("保存字体")?);
    assert!(!app.screen().enabled("label:已安装的字体名称")?);
    crate::platform::STORE
        .lock()
        .expect("test store")
        .write_gate = None;
    gate.notify_one();
    app.wait_for("state=Ready")?;
    assert_font(&app, "JetBrains Mono", 14.0)?;
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    {
        let store = crate::platform::STORE.lock().expect("test store");
        let saved = Preferences::decode(store.value.as_deref(), catalog)
            .map_err(|error| error.to_string())?;
        assert_eq!(saved.font_family, "JetBrains Mono");
        assert_eq!(saved.theme_id, catalog.defaults.theme_id);
        assert_eq!(store.writes, 1);
    }
    drop(app);
    let mut restored = self::app();
    restored.wait_for("state=Ready")?;
    assert_font(&restored, "JetBrains Mono", 14.0)?;
    assert_eq!(
        restored.screen().input_text("label:已安装的字体名称")?,
        Some("JetBrains Mono")
    );
    restored.press("label:终端字体")?;
    restored.press(font::MAPLE)?;
    restored.wait_for("state=Ready")?;
    assert_font(&restored, font::MAPLE, 14.0)?;
    restored.press("label:终端字体")?;
    restored.press("系统等宽字体")?;
    restored.wait_for("state=Ready")?;
    assert_font(&restored, font::SYSTEM, 14.0)?;
    Ok(())
}

#[test]
fn terminal_preferences_font_failures_preserve_saved_family_size_and_other_settings()
-> Result<(), String> {
    use crate::preferences::font;
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    let mut initial = Preferences::defaults(catalog);
    initial.theme_id = "light".into();
    initial.appearance = Appearance::Dark;
    initial.default_backend = neko_remoted_protocol::messages::SessionBackend::Herdr;
    let encoded = initial.encode(catalog).map_err(|error| error.to_string())?;
    reset(Some(encoded.clone()));
    let mut app = app();
    app.wait_for("state=Ready")?;
    crate::platform::STORE
        .lock()
        .expect("test store")
        .fail_write = true;
    app.clear("label:已安装的字体名称")?;
    app.type_text("label:已安装的字体名称", "Iosevka")?;
    app.press("保存字体")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    assert_font(&app, font::HACK, 14.0)?;
    app.press("增大终端字号")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    assert_font(&app, font::HACK, 14.0)?;
    {
        let store = crate::platform::STORE.lock().expect("test store");
        assert_eq!(store.value.as_deref(), Some(encoded.as_str()));
        assert_eq!(store.writes, 0);
    }
    crate::platform::STORE
        .lock()
        .expect("test store")
        .fail_write = false;
    let gate = std::sync::Arc::new(tokio::sync::Notify::new());
    crate::platform::STORE
        .lock()
        .expect("test store")
        .write_gate = Some(gate.clone());
    app.press("增大终端字号")?;
    app.wait_for("state=Saving")?;
    assert_font(&app, font::HACK, 14.0)?;
    crate::platform::STORE
        .lock()
        .expect("test store")
        .write_gate = None;
    gate.notify_one();
    app.wait_for("state=Ready")?;
    assert_font(&app, font::HACK, 15.0)?;
    // An unrelated save must not erase an unsaved custom font edit.
    assert_eq!(
        app.screen().input_text("label:已安装的字体名称")?,
        Some("Iosevka")
    );
    app.press("保存字体")?;
    app.wait_for("state=Ready")?;
    assert_font(&app, "Iosevka", 15.0)?;
    let store = crate::platform::STORE.lock().expect("test store");
    let mut expected = initial;
    expected.font_family = "Iosevka".into();
    expected.font_size = 15;
    assert_eq!(
        Preferences::decode(store.value.as_deref(), catalog).map_err(|error| error.to_string())?,
        expected
    );
    Ok(())
}

#[test]
fn terminal_preferences_invalid_custom_font_never_saves_and_android_only_lists_bundled_fonts()
-> Result<(), String> {
    use crate::preferences::font;
    reset(None);
    let mut app = app();
    app.wait_for("state=Ready")?;
    app.clear("label:已安装的字体名称")?;
    assert!(!app.screen().enabled("保存字体")?);
    app.type_text("label:已安装的字体名称", "Hack, monospace")?;
    app.wait_for("请输入单个字体名称，不含逗号、引号或反斜杠。")?;
    assert!(!app.screen().enabled("保存字体")?);
    app.submit("label:已安装的字体名称")?;
    assert_eq!(crate::platform::STORE.lock().expect("test store").writes, 0);
    assert_font(&app, font::HACK, 14.0)?;
    drop(app);
    let mut android = app_for_os(nsh_core::platform::Os::Android);
    android.wait_for("state=Ready")?;
    assert!(!android.shows("已安装的字体名称"));
    assert!(!android.shows("保存字体"));
    android.press("label:终端字体")?;
    assert!(!android.shows("系统等宽字体"));
    android.press(font::MAPLE)?;
    android.wait_for("state=Ready")?;
    assert_font(&android, font::MAPLE, 14.0)?;
    Ok(())
}

#[test]
fn terminal_preferences_cursor_and_control_changes_wait_for_save_and_keep_manual_access_on_failure()
-> Result<(), String> {
    reset(None);
    let mut app = app();
    app.wait_for("state=Ready")?;
    app.wait_for("cursor=block;blink=false;auto_control=false")?;
    let gate = std::sync::Arc::new(tokio::sync::Notify::new());
    crate::platform::STORE
        .lock()
        .expect("test store")
        .write_gate = Some(gate.clone());
    app.press("label:光标形状")?;
    app.press("竖线")?;
    app.wait_for("state=Saving")?;
    app.wait_for("cursor=block;blink=false;auto_control=false")?;
    crate::platform::STORE
        .lock()
        .expect("test store")
        .write_gate = None;
    gate.notify_one();
    app.wait_for("cursor=beam;blink=false;auto_control=false")?;
    app.press("label:光标形状")?;
    app.press("下划线")?;
    app.wait_for("cursor=underline;blink=false;auto_control=false")?;
    app.press("label:光标闪烁")?;
    app.press("闪烁")?;
    app.wait_for("cursor=underline;blink=true;auto_control=false")?;
    crate::platform::STORE
        .lock()
        .expect("test store")
        .fail_write = true;
    app.press("label:光标形状")?;
    app.press("方块")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    app.wait_for("cursor=underline;blink=true;auto_control=false")?;
    app.press("label:远程终端打开时自动取得控制权")?;
    app.press("打开时自动请求")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    app.wait_for("cursor=underline;blink=true;auto_control=false")?;
    crate::platform::STORE
        .lock()
        .expect("test store")
        .fail_write = false;
    app.press("label:远程终端打开时自动取得控制权")?;
    app.press("打开时自动请求")?;
    app.wait_for("cursor=underline;blink=true;auto_control=true")?;
    app.wait_for(
        "仅为每个打开的远程终端请求控制权，不会控制整台主机；只读权限或主机拒绝时仍保持只读。",
    )?;
    drop(app);
    let mut restored = self::app();
    restored.wait_for("cursor=underline;blink=true;auto_control=true")?;
    Ok(())
}

#[test]
fn terminal_preferences_window_close_choice_is_desktop_only_and_acknowledged() -> Result<(), String>
{
    reset(None);
    let mut app = app();
    app.wait_for("state=Ready")?;
    app.wait_for("close=ask")?;
    for (label, value) in [
        ("最小化到托盘", "tray"),
        ("退出应用", "quit"),
        ("每次询问", "ask"),
    ] {
        app.press("label:关闭窗口时")?;
        app.press(label)?;
        app.wait_for(&format!("close={value}"))?;
    }
    crate::platform::STORE
        .lock()
        .expect("test store")
        .fail_write = true;
    app.press("label:关闭窗口时")?;
    app.press("最小化到托盘")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    app.wait_for("close=ask")?;
    drop(app);
    let mut android = app_for_os(nsh_core::platform::Os::Android);
    android.wait_for("close=ask")?;
    assert!(!android.shows("关闭窗口时"));
    assert!(!android.shows("最小化到托盘"));
    Ok(())
}

fn reset(value: Option<String>) {
    *crate::platform::STORE.lock().expect("test store") = crate::platform::Memory {
        value,
        ..Default::default()
    };
}

fn app() -> Headless {
    app_for_os(nsh_core::platform::Os::Linux)
}

fn app_for_os(os: nsh_core::platform::Os) -> Headless {
    let mut environment = Environment::default();
    environment.os = os;
    environment.locale = nsh_core::platform::Locale::parse("zh-CN").expect("Chinese locale");
    Headless::start(EngineConfig {
        environment,
        // The host supplies scrolling; this fixture exposes the complete settings document.
        viewport: Size::new(px(900.0), px(2200.0)),
        ..EngineConfig::new(Root::of::<Fixture>().tokens(
            nsh_base::tokens_of(Some(include_str!("../ui/theme.json"))).expect("app tokens"),
        ))
    })
    .expect("headless engine")
}

#[test]
fn settings_save_and_restore_with_translated_existing_bindings() -> Result<(), String> {
    reset(None);
    let mut app = app();
    app.wait_for("更改自动保存并生效")?;
    app.wait_for("终端")?;
    app.wait_for("已退出（3）")?;
    app.press("Apply fixture")?;
    app.wait_for("Changes are saved and applied automatically")?;
    app.wait_for("Host 7")?;
    app.wait_for("Exited (3)")?;
    app.wait_for("Terminal")?;
    let preview = app
        .screen()
        .span_style("neko@host:~$ printf 'Hello, 世界'")?;
    assert_eq!(preview.font_size.0, 20.0);
    assert_ne!(preview.color, app.screen().span_style("Preferences")?.color);
    let saved = {
        let store = crate::platform::STORE.lock().expect("test store");
        assert_eq!(store.writes, 1);
        store.value.clone().expect("saved preferences")
    };
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    let preferences =
        Preferences::decode(Some(&saved), catalog).map_err(|error| error.to_string())?;
    assert_eq!(preferences.font_size, 20);
    assert_eq!(preferences.theme_id, "light");
    assert_eq!(preferences.language, Language::EnUs);
    drop(app);
    let mut restored = self::app();
    restored.wait_for("Host 7")?;
    restored.wait_for("20")?;
    assert_eq!(crate::platform::STORE.lock().expect("test store").writes, 1);
    Ok(())
}

#[test]
fn failed_load_blocks_writes_and_retry_recovers() -> Result<(), String> {
    reset(Some("corrupt".into()));
    let mut app = app();
    app.wait_for("text:/^state=LoadFailed/")?;
    app.press("Apply fixture")?;
    assert_eq!(crate::platform::STORE.lock().expect("test store").writes, 0);
    crate::platform::STORE.lock().expect("test store").value = None;
    app.press("重试")?;
    app.wait_for("更改自动保存并生效")?;
    Ok(())
}

#[test]
fn failed_save_keeps_previous_values_and_allows_retry() -> Result<(), String> {
    reset(None);
    let mut app = app();
    app.wait_for("更改自动保存并生效")?;
    crate::platform::STORE
        .lock()
        .expect("test store")
        .fail_write = true;
    app.press("Apply fixture")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    app.wait_for("14")?;
    assert_eq!(crate::platform::STORE.lock().expect("test store").writes, 0);
    crate::platform::STORE
        .lock()
        .expect("test store")
        .fail_write = false;
    app.press("Apply fixture")?;
    app.wait_for("Host 7")?;
    Ok(())
}

#[test]
fn following_system_releases_explicit_locale_and_appearance() -> Result<(), String> {
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    let mut saved = Preferences::defaults(catalog);
    saved.language = Language::EnUs;
    saved.appearance = Appearance::Dark;
    reset(Some(
        saved.encode(catalog).map_err(|error| error.to_string())?,
    ));
    let mut app = app();
    app.wait_for("Host 7")?;
    app.press("Follow system")?;
    app.wait_for("主机 7")?;
    let mut environment = Environment::default();
    environment.locale = nsh_core::platform::Locale::EN_US;
    environment.color_scheme = nsh_core::platform::ColorScheme::Dark;
    app.set_environment("change system locale and appearance", |current| {
        *current = environment
    })?;
    app.wait_for("Host 7")?;
    app.wait_for("text:/^font=14; appearance=\"Dark\"/")?;
    let saved = crate::platform::STORE
        .lock()
        .expect("test store")
        .value
        .clone();
    let saved =
        Preferences::decode(saved.as_deref(), catalog).map_err(|error| error.to_string())?;
    assert_eq!(saved.language, Language::System);
    assert_eq!(saved.appearance, Appearance::System);
    Ok(())
}

#[test]
fn preference_controls_save_font_and_searchable_theme() -> Result<(), String> {
    reset(None);
    let mut app = app();
    app.wait_for("更改自动保存并生效")?;
    app.press("增大终端字号")?;
    app.wait_for("text:/^font=15;/")?;
    assert_eq!(
        app.screen()
            .span_style("neko@host:~$ printf 'Hello, 世界'")?
            .font_size
            .0,
        15.0
    );
    app.press("label:终端主题")?;
    app.type_text("label:搜索主题", "Classic Light")?;
    app.wait_for("Classic Light")?;
    app.press("Classic Light")?;
    app.wait_for("theme=light")?;
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    let store = crate::platform::STORE.lock().expect("test store");
    let saved =
        Preferences::decode(store.value.as_deref(), catalog).map_err(|error| error.to_string())?;
    assert_eq!(saved.font_size, 15);
    assert_eq!(saved.theme_id, "light");
    assert_eq!(store.writes, 2);
    Ok(())
}

#[component]
fn LoadedFixture() -> impl IntoElement {
    let catalog = Catalog::shared().expect("test catalog");
    let mut saved = Preferences::defaults(catalog);
    saved.language = Language::EnUs;
    saved.appearance = Appearance::Dark;
    let ui = Ui::from_loaded(saved).expect("validated startup preferences");
    view()
        .child(text(move || preferences::tr("终端")))
        .child(text(move || format!("{:?}", ui.state())))
        .child(text(move || if dark() { "dark" } else { "light" }))
}

#[test]
fn loaded_startup_installs_context_without_async_storage_read() -> Result<(), String> {
    reset(None);
    crate::platform::STORE.lock().expect("test store").fail_read = true;
    let mut app = Headless::start(EngineConfig::new(Root::of::<LoadedFixture>()))
        .map_err(|error| error.to_string())?;
    app.wait_for("Terminal")?;
    app.wait_for("Ready")?;
    app.wait_for("dark")?;
    assert_eq!(crate::platform::STORE.lock().expect("test store").writes, 0);
    Ok(())
}

#[test]
fn default_backend_stays_unavailable_until_the_saved_preferences_load() -> Result<(), String> {
    use neko_remoted_protocol::messages::SessionBackend;
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    let mut saved = Preferences::defaults(catalog);
    saved.default_backend = SessionBackend::Herdr;
    reset(Some(
        saved.encode(catalog).map_err(|error| error.to_string())?,
    ));
    let gate = std::sync::Arc::new(tokio::sync::Notify::new());
    crate::platform::STORE.lock().unwrap().read_gate = Some(gate.clone());
    let mut app = app();
    app.wait_for("state=Loading")?;
    app.wait_for("backend=None")?;
    assert!(!app.screen().enabled("label:启动终端类型")?);
    crate::platform::STORE.lock().unwrap().read_gate = None;
    gate.notify_one();
    app.wait_for("backend=Some(Herdr)")?;
    app.wait_for("state=Ready")?;
    assert!(app.screen().enabled("label:启动终端类型")?);
    assert_eq!(crate::platform::STORE.lock().unwrap().writes, 0);
    Ok(())
}

#[test]
fn default_backend_changes_only_after_save_acknowledgment_and_restores() -> Result<(), String> {
    use neko_remoted_protocol::messages::SessionBackend;
    reset(None);
    let mut app = app();
    app.wait_for("backend=Some(Native)")?;
    let gate = std::sync::Arc::new(tokio::sync::Notify::new());
    crate::platform::STORE.lock().unwrap().write_gate = Some(gate.clone());
    app.press("label:启动终端类型")?;
    app.press("hc（herdr-core）")?;
    app.wait_for("state=Saving")?;
    app.wait_for("backend=Some(Native)")?;
    assert!(!app.screen().enabled("label:启动终端类型")?);
    assert_eq!(crate::platform::STORE.lock().unwrap().writes, 0);
    crate::platform::STORE.lock().unwrap().write_gate = None;
    gate.notify_one();
    app.wait_for("backend=Some(Hc)")?;
    app.wait_for("state=Ready")?;
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    {
        let store = crate::platform::STORE.lock().unwrap();
        let saved = Preferences::decode(store.value.as_deref(), catalog)
            .map_err(|error| error.to_string())?;
        assert_eq!(saved.default_backend, SessionBackend::Hc);
        assert_eq!(store.writes, 1);
    }
    drop(app);
    let mut restored = self::app();
    restored.wait_for("backend=Some(Hc)")?;
    for (label, backend) in [
        ("Herdr", "backend=Some(Herdr)"),
        ("原生终端", "backend=Some(Native)"),
    ] {
        restored.press("label:启动终端类型")?;
        restored.press(label)?;
        restored.wait_for(backend)?;
        restored.wait_for("state=Ready")?;
    }
    assert_eq!(crate::platform::STORE.lock().unwrap().writes, 3);
    Ok(())
}

#[test]
fn failed_default_backend_save_keeps_last_confirmed_backend_and_allows_retry() -> Result<(), String>
{
    reset(None);
    let mut app = app();
    app.wait_for("backend=Some(Native)")?;
    crate::platform::STORE.lock().unwrap().fail_write = true;
    app.press("label:启动终端类型")?;
    app.press("Herdr")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    app.wait_for("backend=Some(Native)")?;
    assert_eq!(crate::platform::STORE.lock().unwrap().writes, 0);
    crate::platform::STORE.lock().unwrap().fail_write = false;
    app.press("label:启动终端类型")?;
    app.press("Herdr")?;
    app.wait_for("backend=Some(Herdr)")?;
    app.wait_for("state=Ready")?;
    Ok(())
}

#[test]
fn bad_saved_backend_blocks_creation_and_never_overwrites_storage() -> Result<(), String> {
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    let mut saved =
        serde_json::to_value(Preferences::defaults(catalog)).map_err(|error| error.to_string())?;
    saved["default_backend"] = serde_json::json!("unsupported");
    let source = saved.to_string();
    reset(Some(source.clone()));
    let mut app = app();
    app.wait_for("text:/^state=LoadFailed/")?;
    app.wait_for("backend=None")?;
    assert!(!app.screen().enabled("label:启动终端类型")?);
    let store = crate::platform::STORE.lock().unwrap();
    assert_eq!(store.value.as_deref(), Some(source.as_str()));
    assert_eq!(store.writes, 0);
    Ok(())
}

#[test]
fn android_preferences_hide_the_local_default_backend_without_losing_other_settings()
-> Result<(), String> {
    reset(None);
    let mut app = app_for_os(nsh_core::platform::Os::Android);
    app.wait_for("更改自动保存并生效")?;
    assert!(!app.shows("启动终端类型"));
    assert!(!app.shows("仅用于本机新建会话，不会更改已有会话。"));
    app.wait_for("终端主题")?;
    app.wait_for("语言")?;
    assert_eq!(crate::platform::STORE.lock().unwrap().writes, 0);
    Ok(())
}

#[test]
fn backend_choice_translates_the_selected_title_and_options_without_changing_storage_values()
-> Result<(), String> {
    use neko_remoted_protocol::messages::SessionBackend;
    reset(None);
    let mut app = app();
    app.wait_for("更改自动保存并生效")?;
    app.wait_for("原生终端")?;
    app.press("label:启动终端类型")?;
    app.wait_for("hc（herdr-core）")?;
    app.wait_for("Herdr")?;
    app.press("hc（herdr-core）")?;
    app.wait_for("backend=Some(Hc)")?;
    app.press("label:语言")?;
    app.press("English (US)")?;
    app.wait_for("Startup terminal type")?;
    app.wait_for("hc (herdr-core)")?;
    assert!(!app.shows("hc（herdr-core）"));
    app.press("label:Startup terminal type")?;
    app.wait_for("Native terminal")?;
    app.wait_for("Herdr")?;
    assert!(!app.shows("原生终端"));
    app.press("Native terminal")?;
    app.wait_for("backend=Some(Native)")?;
    app.wait_for("state=Ready")?;
    let catalog = Catalog::shared().map_err(|error| error.to_string())?;
    let store = crate::platform::STORE.lock().unwrap();
    let saved =
        Preferences::decode(store.value.as_deref(), catalog).map_err(|error| error.to_string())?;
    assert_eq!(saved.default_backend, SessionBackend::Native);
    assert_eq!(saved.language, Language::EnUs);
    Ok(())
}
