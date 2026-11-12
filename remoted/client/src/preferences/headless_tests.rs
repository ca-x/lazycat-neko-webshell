//! Standalone integration tests loaded by check.mjs, without modifying the host module tree.

use crate::preferences::{self, Appearance, Language, Preferences, Ui, catalog::Catalog};
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Environment, Root};
use nsh_testing::Headless;

#[component]
fn Fixture() -> impl IntoElement {
    let ui = Ui::start().expect("valid test catalog");
    view()
        .child(text(move || {
            format!("backend={:?}", ui.new_session_backend())
        }))
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
        .child(preferences::current_context().terminal().h(px(100.0)))
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
    assert!(!app.screen().enabled("label:默认会话后端")?);
    crate::platform::STORE.lock().unwrap().read_gate = None;
    gate.notify_one();
    app.wait_for("backend=Some(Herdr)")?;
    app.wait_for("state=Ready")?;
    assert!(app.screen().enabled("label:默认会话后端")?);
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
    app.press("label:默认会话后端")?;
    app.press("hc（herdr-core）")?;
    app.wait_for("state=Saving")?;
    app.wait_for("backend=Some(Native)")?;
    assert!(!app.screen().enabled("label:默认会话后端")?);
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
        restored.press("label:默认会话后端")?;
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
    app.press("label:默认会话后端")?;
    app.press("Herdr")?;
    app.wait_for("text:/^state=SaveFailed/")?;
    app.wait_for("backend=Some(Native)")?;
    assert_eq!(crate::platform::STORE.lock().unwrap().writes, 0);
    crate::platform::STORE.lock().unwrap().fail_write = false;
    app.press("label:默认会话后端")?;
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
    assert!(!app.screen().enabled("label:默认会话后端")?);
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
    assert!(!app.shows("默认会话后端"));
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
    app.wait_for("原生终端")?;
    app.press("label:默认会话后端")?;
    app.wait_for("hc（herdr-core）")?;
    app.wait_for("Herdr")?;
    app.press("hc（herdr-core）")?;
    app.wait_for("backend=Some(Hc)")?;
    app.press("label:语言")?;
    app.press("English (US)")?;
    app.wait_for("Default session backend")?;
    app.wait_for("hc (herdr-core)")?;
    assert!(!app.shows("hc（herdr-core）"));
    app.press("label:Default session backend")?;
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
