//! Explicit native translations; existing hardcoded host UI must opt in to these keys.

use serde::{Deserialize, Serialize};
#[path = "literals.rs"]
mod literals;

/// Persisted language override, separate from the effective platform locale.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Language {
    /// Follow the platform locale and future changes.
    #[default]
    #[serde(rename = "system")]
    System,
    /// Simplified Chinese.
    #[serde(rename = "zh-CN")]
    ZhCn,
    /// US English, also used for unsupported system languages.
    #[serde(rename = "en-US")]
    EnUs,
}

impl Language {
    /// Resolve Chinese system locales to Chinese, and other languages to English.
    pub fn resolve(self, system_locale: &str) -> Self {
        match self {
            Self::System
                if system_locale
                    .split(['-', '_'])
                    .next()
                    .is_some_and(|language| language.eq_ignore_ascii_case("zh")) =>
            {
                Self::ZhCn
            }
            Self::System => Self::EnUs,
            explicit => explicit,
        }
    }

    pub(crate) fn native(self) -> Option<nsh_core::platform::Locale> {
        match self {
            Self::System => None,
            Self::ZhCn => nsh_core::platform::Locale::parse("zh-CN"),
            Self::EnUs => Some(nsh_core::platform::Locale::EN_US),
        }
    }
}

/// One shared vocabulary entry usable by host navigation and feature modules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Translation {
    /// Stable lookup key.
    pub key: &'static str,
    /// Simplified Chinese copy.
    pub zh_cn: &'static str,
    /// US English copy.
    pub en_us: &'static str,
}

macro_rules! translations {
    ($(($key:literal, $zh:literal, $en:literal)),* $(,)?) => {
        /// Public mapping for replacing host UI literals incrementally.
        pub const TRANSLATIONS: &[Translation] = &[$(Translation { key: $key, zh_cn: $zh, en_us: $en }),*];
    };
}

translations! {
    ("preferences.title", "偏好设置", "Preferences"),
    ("preferences.theme", "终端主题", "Terminal theme"),
    ("preferences.font_size", "终端字号", "Terminal font size"),
    ("preferences.appearance", "界面外观", "Appearance"),
    ("preferences.language", "语言", "Language"),
    ("preferences.default_backend", "默认会话后端", "Default session backend"),
    ("preferences.backend_native", "原生终端", "Native terminal"),
    ("preferences.backend_hc", "hc（herdr-core）", "hc (herdr-core)"),
    ("preferences.backend_herdr", "Herdr", "Herdr"),
    ("preferences.default_backend_hint", "仅用于本机新建会话，不会更改已有会话。", "Used only for new local sessions; existing sessions keep their backend."),
    ("preferences.system", "跟随系统", "System"),
    ("preferences.light", "浅色", "Light"),
    ("preferences.dark", "深色", "Dark"),
    ("preferences.search", "搜索主题", "Search themes"),
    ("preferences.empty", "没有匹配的主题", "No matching themes"),
    ("preferences.loading", "正在读取偏好…", "Loading preferences…"),
    ("preferences.saving", "正在保存…", "Saving…"),
    ("preferences.saved", "更改自动保存并生效", "Changes are saved and applied automatically"),
    ("preferences.load_failed", "无法读取偏好", "Could not load preferences"),
    ("preferences.save_failed", "保存失败，仍使用上次保存的设置", "Could not save; previous settings remain active"),
    ("preferences.retry", "重试", "Retry"),
    ("preferences.preview", "终端预览", "Terminal preview"),
    ("preferences.smaller", "减小终端字号", "Decrease terminal font size"),
    ("preferences.larger", "增大终端字号", "Increase terminal font size"),
    ("nav.terminal", "终端", "Terminal"),
    ("nav.herdr", "Herdr", "Herdr"),
    ("nav.files", "文件", "Files"),
    ("nav.settings", "设置", "Settings"),
    ("nav.devices", "设备", "Devices"),
    ("action.connect", "连接", "Connect"),
    ("action.disconnect", "断开", "Disconnect"),
    ("action.cancel", "取消", "Cancel"),
    ("action.close", "关闭", "Close"),
    ("action.back", "返回", "Back"),
    ("action.refresh", "刷新", "Refresh"),
    ("status.read_only", "只读", "Read only"),
    ("status.offline", "离线", "Offline"),
    ("status.connecting", "连接中…", "Connecting…"),
    ("终端模式", "终端模式", "Terminal mode"),
    ("对话模式", "对话模式", "Conversation mode"),
    ("真实终端输出", "真实终端输出", "Live terminal output"),
    ("你提交的文本", "你提交的文本", "Your submitted text"),
    ("等待主机确认输入…", "等待主机确认输入…", "Waiting for host acknowledgment…"),
    ("主机已接受输入", "主机已接受输入", "Host accepted the input"),
    ("输入未被接受", "输入未被接受", "Input was not accepted"),
    ("输入结果未知；未自动重发", "输入结果未知；未自动重发", "Input outcome unknown; not resent automatically"),
}

/// Translate a key with an explicit system locale; unknown keys remain visible.
pub fn tr<'a>(language: Language, system_locale: &str, key: &'a str) -> &'a str {
    let chinese = language.resolve(system_locale) == Language::ZhCn;
    if let Some(entry) = TRANSLATIONS
        .iter()
        .find(|entry| entry.key == key || entry.zh_cn == key)
    {
        return if chinese { entry.zh_cn } else { entry.en_us };
    }
    if !chinese
        && let Some((_, english)) = literals::LITERALS.iter().find(|(source, _)| *source == key)
    {
        return english;
    }
    key
}

/// Translate known formatted UI statuses while preserving captured display values.
///
/// Only catalogued templates with a nonempty literal prefix are recognized.
/// Prefer `tr_format` at call sites that still have the original values available.
pub(crate) fn translate(language: Language, system_locale: &str, source: &str) -> String {
    let translated = tr(language, system_locale, source);
    if translated != source || language.resolve(system_locale) == Language::ZhCn {
        return translated.to_owned();
    }
    for &(template, english) in literals::LITERALS {
        if !template.contains("{}") || template.starts_with("{}") {
            continue;
        }
        if let Some(values) = capture(template, source) {
            let mut parts = english.split("{}");
            let mut result = parts.next().unwrap_or_default().to_owned();
            for (value, suffix) in values.into_iter().zip(parts) {
                result.push_str(value);
                result.push_str(suffix);
            }
            return result;
        }
    }
    source.to_owned()
}

fn capture<'a>(template: &str, source: &'a str) -> Option<Vec<&'a str>> {
    let mut parts = template.split("{}");
    let mut remaining = source.strip_prefix(parts.next()?)?;
    let suffixes: Vec<_> = parts.collect();
    let mut values = Vec::with_capacity(suffixes.len());
    for (index, suffix) in suffixes.iter().enumerate() {
        if index + 1 == suffixes.len() {
            values.push(remaining.strip_suffix(suffix)?);
            remaining = "";
        } else {
            let end = remaining.find(suffix)?;
            values.push(&remaining[..end]);
            remaining = &remaining[end + suffix.len()..];
        }
    }
    remaining.is_empty().then_some(values)
}

#[cfg(test)]
pub(crate) fn verify_literals() {
    let mut sources = std::collections::HashSet::new();
    for &(source, english) in literals::LITERALS {
        assert!(sources.insert(source), "duplicate literal {source}");
        assert_eq!(
            source.matches("{}").count(),
            english.matches("{}").count(),
            "placeholder mismatch for {source}"
        );
    }
}
