use super::*;
use nsh_base::{TerminalColor, TerminalLine, TerminalRun, TerminalStyle};

#[test]
fn catalog_covers_web_inkstone_and_restty_with_licenses() -> Result<()> {
    let catalog = catalog::Catalog::shared()?;
    for id in [
        "ghostty",
        "catppuccin-mocha",
        "light",
        "inkstone-xuanmo",
        "ghostty:Catppuccin Mocha",
    ] {
        assert!(catalog.find(id).is_some(), "missing theme {id}");
    }
    assert!(catalog.themes.len() > 450);
    for license in &catalog.licenses {
        assert!(license.text.contains("Permission is hereby granted"));
        assert!(license.text.contains("Copyright"));
    }
    Ok(())
}

#[test]
fn preferences_round_trip_and_preserve_independent_choices() -> Result<()> {
    let catalog = catalog::Catalog::shared()?;
    let mut preferences = Preferences::defaults(catalog);
    preferences.theme_id = "inkstone-xuanmo".into();
    preferences.font_size = 22;
    preferences.appearance = Appearance::Light;
    preferences.language = Language::EnUs;
    let encoded = preferences.encode(catalog)?;
    assert_eq!(Preferences::decode(Some(&encoded), catalog)?, preferences);
    assert!(encoded.contains("en-US"));
    assert_eq!(
        Preferences::decode(None, catalog)?,
        Preferences::defaults(catalog)
    );
    Ok(())
}

#[test]
fn damaged_unknown_or_future_documents_are_not_defaults() -> Result<()> {
    let catalog = catalog::Catalog::shared()?;
    for document in ["", "{", "null", r#"{"version":99}"#] {
        assert!(
            Preferences::decode(Some(document), catalog).is_err(),
            "accepted {document}"
        );
    }
    let mut preferences = Preferences::defaults(catalog);
    preferences.theme_id = "missing-theme".into();
    assert!(preferences.encode(catalog).is_err());
    preferences.theme_id = catalog.defaults.theme_id.clone();
    preferences.version = 2;
    assert!(preferences.encode(catalog).is_err());
    preferences.version = 1;
    for size in [0, MIN_FONT_SIZE - 1, MAX_FONT_SIZE + 1, u8::MAX] {
        preferences.font_size = size;
        assert!(
            preferences.encode(catalog).is_err(),
            "accepted font size {size}"
        );
    }
    Ok(())
}

#[test]
fn system_language_is_resolved_without_persisting_a_snapshot() {
    for locale in ["zh-CN", "zh_Hans_CN", "ZH-TW"] {
        assert_eq!(locale::tr(Language::System, locale, "nav.terminal"), "终端");
    }
    for locale in ["en-US", "de-DE", ""] {
        assert_eq!(locale::tr(Language::System, locale, "终端"), "Terminal");
    }
    assert_eq!(locale::tr(Language::ZhCn, "en-US", "nav.terminal"), "终端");
    assert_eq!(locale::tr(Language::EnUs, "zh-CN", "终端"), "Terminal");
    assert_eq!(locale::tr(Language::EnUs, "zh-CN", "主机 {}"), "Host {}");
    assert_eq!(
        locale::tr(Language::EnUs, "zh-CN", "User-owned name"),
        "User-owned name"
    );
}

#[test]
fn translation_keys_and_literal_sources_are_unambiguous() {
    let mut keys = std::collections::HashSet::new();
    for entry in locale::TRANSLATIONS {
        assert!(
            keys.insert(entry.key),
            "duplicate translation {}",
            entry.key
        );
        assert!(!entry.zh_cn.is_empty() && !entry.en_us.is_empty());
    }
}

#[test]
fn themed_lines_preserve_truecolor_and_resolve_inverse_defaults() -> Result<()> {
    let catalog = catalog::Catalog::shared()?;
    let theme = catalog.find("catppuccin-mocha").expect("shared Web theme");
    let mut style = TerminalStyle::default();
    style.fg = Some(TerminalColor::Indexed(1));
    style.bg = Some(TerminalColor::Rgb(12, 34, 56));
    style.bold = true;
    let line = TerminalLine::new(vec![TerminalRun::new("hello", style)]);
    let mapped = theme.line(line.clone());
    assert_eq!(mapped.text(), line.text());
    assert_eq!(mapped.runs[0].style.bg, style.bg);
    assert!(mapped.runs[0].style.bold);
    assert_eq!(
        mapped.runs[0].style.fg,
        Some(TerminalColor::Rgb(243, 139, 168))
    );
    assert_eq!(line.runs[0].style.fg, Some(TerminalColor::Indexed(1)));
    let mut inverse = TerminalStyle::default();
    inverse.inverse = true;
    let inverse = theme.style(inverse);
    assert!(!inverse.inverse);
    assert_eq!(inverse.fg, Some(TerminalColor::Rgb(30, 30, 46)));
    assert_eq!(inverse.bg, Some(TerminalColor::Rgb(205, 214, 244)));
    Ok(())
}

#[test]
fn catalog_rejects_missing_palette_and_attribution() -> Result<()> {
    let source = include_str!("../../../themes/catalog.json");
    let mut value: serde_json::Value = serde_json::from_str(source)?;
    value["themes"][0]["palette"] = serde_json::json!([0]);
    assert!(catalog::Catalog::parse(&value.to_string()).is_err());
    let mut value: serde_json::Value = serde_json::from_str(source)?;
    value["licenses"] = serde_json::json!([]);
    assert!(catalog::Catalog::parse(&value.to_string()).is_err());
    let mut value: serde_json::Value = serde_json::from_str(source)?;
    value["licenses"][0]["path"] = serde_json::json!("");
    assert!(catalog::Catalog::parse(&value.to_string()).is_err());
    let mut value: serde_json::Value = serde_json::from_str(source)?;
    value["restty_version"] = serde_json::json!("");
    assert!(catalog::Catalog::parse(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn formatted_statuses_keep_names_fingerprints_and_error_details() {
    for (source, expected) in [
        ("主机 7", "Host 7"),
        ("已退出（3）", "Exited (3)"),
        ("结束“项目 / Shell”？", "End “项目 / Shell”?"),
        ("本设备指纹：AB:CD", "This device fingerprint: AB:CD"),
        (
            "读取设备码未完成，可继续手输：读取失败: IO",
            "Could not read device code; you can enter it manually: 读取失败: IO",
        ),
        (
            "已断开，自动连接已停止；主机终端仍保留。 detail",
            "Disconnected; auto-connect stopped. Host terminals remain available. detail",
        ),
        ("设备 a · 主机 b", "Device a · Host b"),
        ("my 主机 7", "my 主机 7"),
        ("项目 / 服务 / Shell", "项目 / 服务 / Shell"),
        ("已退出（3）suffix", "已退出（3）suffix"),
    ] {
        assert_eq!(
            locale::translate(Language::EnUs, "zh-CN", source),
            expected,
            "source {source:?}"
        );
        assert_eq!(locale::translate(Language::ZhCn, "en-US", source), source);
    }
}

#[test]
fn every_literal_has_unique_source_and_matching_placeholder_count() {
    locale::verify_literals();
}

#[test]
fn old_preferences_default_new_sessions_to_native_without_changing_other_fields() -> Result<()> {
    let catalog = catalog::Catalog::shared()?;
    let mut expected = Preferences::defaults(catalog);
    expected.theme_id = "inkstone-xuanmo".into();
    expected.font_size = 22;
    expected.language = Language::EnUs;
    let mut document = serde_json::to_value(&expected)?;
    document.as_object_mut().unwrap().remove("default_backend");
    assert_eq!(
        Preferences::decode(Some(&document.to_string()), catalog)?,
        expected
    );
    Ok(())
}

#[test]
fn new_session_backends_round_trip_as_exact_protocol_values() -> Result<()> {
    let catalog = catalog::Catalog::shared()?;
    for (backend, name) in [
        (SessionBackend::Native, "native"),
        (SessionBackend::Hc, "hc"),
        (SessionBackend::Herdr, "herdr"),
    ] {
        let mut preferences = Preferences::defaults(catalog);
        preferences.default_backend = backend;
        let document = preferences.encode(catalog)?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&document)?["default_backend"],
            name
        );
        assert_eq!(Preferences::decode(Some(&document), catalog)?, preferences);
    }
    Ok(())
}

#[test]
fn invalid_default_backends_are_errors_instead_of_native_defaults() -> Result<()> {
    let catalog = catalog::Catalog::shared()?;
    for invalid in [
        serde_json::json!("unknown"),
        serde_json::json!("Native"),
        serde_json::json!(""),
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!({"native": true}),
    ] {
        let mut document = serde_json::to_value(Preferences::defaults(catalog))?;
        document["default_backend"] = invalid;
        assert!(
            Preferences::decode(Some(&document.to_string()), catalog).is_err(),
            "accepted {document}"
        );
    }
    Ok(())
}

#[test]
fn new_session_labels_and_preference_copy_are_bilingual() {
    for (chinese, english) in [
        ("新建终端", "New terminal"),
        ("新建 hc 终端", "New hc terminal"),
        ("新建 Herdr 会话", "New Herdr session"),
        ("会话连接已中断。", "Session connection interrupted."),
        (
            "会话连接已中断。关闭此标签后，可从会话列表重新打开。",
            "The session connection was interrupted. Close this tab and reopen it from the session list.",
        ),
        ("本机 hc", "Local hc"),
        ("刷新 hc 状态", "Refresh hc status"),
        (
            "hc 尚未连接。请检测安装状态后连接。",
            "hc is not connected. Check installation, then connect.",
        ),
        (
            "未安装 hc。请按官方说明安装，然后刷新状态。",
            "hc is not installed. Follow the official guide, then refresh.",
        ),
        (
            "hc 已连接，可创建持久化终端。",
            "hc is connected. You can create persistent terminals.",
        ),
        (
            "hc 连接失败，请查看错误后重试。",
            "Could not connect to hc. Check the error and retry.",
        ),
        ("hc 状态暂不可用。", "hc status is unavailable."),
        ("连接 hc", "Connect hc"),
        ("hc 官方安装指南", "Official hc installation guide"),
        ("默认会话后端", "Default session backend"),
        ("原生终端", "Native terminal"),
        ("hc（herdr-core）", "hc (herdr-core)"),
        (
            "仅用于本机新建会话，不会更改已有会话。",
            "Used only for new local sessions; existing sessions keep their backend.",
        ),
    ] {
        assert_eq!(locale::tr(Language::EnUs, "zh-CN", chinese), english);
        assert_eq!(locale::tr(Language::ZhCn, "en-US", chinese), chinese);
    }
}
