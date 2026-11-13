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
    ("app.name", "小橘终端", "neoko-terminal"),
    ("preferences.title", "偏好设置", "Preferences"),
    ("preferences.theme", "终端主题", "Terminal theme"),
    ("preferences.font_size", "终端字号", "Terminal font size"),
    ("preferences.font_family", "终端字体", "Terminal font family"),
    ("preferences.font_system", "系统等宽字体", "System monospace"),
    ("preferences.font_custom", "已安装的字体名称", "Installed font family"),
    ("preferences.font_custom_hint", "输入本设备已安装的字体名称；不可用时使用内置字体。", "Enter a font family installed on this device. Bundled fonts are used when unavailable."),
    ("preferences.font_save", "保存字体", "Save font family"),
    ("preferences.font_name_required", "请输入不含首尾空格的字体名称。", "Enter a font family without leading or trailing spaces."),
    ("preferences.font_name_length", "字体名称最多为 128 字节。", "Font family names must be at most 128 bytes."),
    ("preferences.font_name_controls", "字体名称不能包含换行或控制字符。", "Font family names cannot contain line breaks or control characters."),
    ("preferences.font_name_single", "请输入单个字体名称，不含逗号、引号或反斜杠。", "Enter one font family without commas, quotes or backslashes."),
    ("preferences.cursor_style", "光标形状", "Cursor shape"),
    ("preferences.cursor_block", "方块", "Block"),
    ("preferences.cursor_beam", "竖线", "Beam"),
    ("preferences.cursor_underline", "下划线", "Underline"),
    ("preferences.cursor_blink", "光标闪烁", "Cursor blinking"),
    ("preferences.cursor_steady", "不闪烁", "Steady"),
    ("preferences.cursor_blinking", "闪烁", "Blinking"),
    ("preferences.remote_auto_control", "远程终端打开时自动取得控制权", "Request control when opening a remote terminal"),
    ("preferences.remote_control_manual", "手动取得控制权", "Request manually"),
    ("preferences.remote_control_auto", "打开时自动请求", "Request on open"),
    ("preferences.remote_auto_control_hint", "仅为每个打开的远程终端请求控制权，不会控制整台主机；只读权限或主机拒绝时仍保持只读。", "Requests control of each remote terminal opened, not the whole host. Read-only permissions or a denied request keep the terminal read-only."),
    ("preferences.close_behavior", "关闭窗口时", "When closing the window"),
    ("preferences.close_ask", "每次询问", "Ask each time"),
    ("preferences.close_quit", "退出应用", "Quit the application"),
    ("preferences.close_tray", "最小化到托盘", "Minimize to tray"),
    ("preferences.appearance", "界面外观", "Appearance"),
    ("preferences.language", "语言", "Language"),
    ("preferences.default_backend", "启动终端类型", "Startup terminal type"),
    ("preferences.backend_native", "原生终端", "Native terminal"),
    ("preferences.backend_hc", "hc（herdr-core）", "hc (herdr-core)"),
    ("preferences.backend_herdr", "Herdr", "Herdr"),
    ("preferences.default_backend_hint", "仅决定进入应用或本机工作区时的初始终端类型；新建遵循当前选择的类型。", "Selects the initial terminal type when entering the app or local workspace. New terminals use the currently selected type."),
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
    ("重命名", "重命名", "Rename"),
    ("可输入", "可输入", "Writable"),
    ("没有可用于新建终端的已授权目标。", "没有可用于新建终端的已授权目标。", "No authorized target is available for creating a terminal."),
    ("status.read_only", "只读", "Read only"),
    ("status.offline", "离线", "Offline"),
    ("status.connecting", "连接中…", "Connecting…"),
    ("远程主机 · {}", "远程主机 · {}", "Remote host · {}"),
    ("主机别名", "主机别名", "Host alias"),
    ("仅保存在此客户端", "仅保存在此客户端", "Saved only on this client"),
    ("留空恢复默认名称。", "留空恢复默认名称。", "Leave blank to restore the default name."),
    ("保存别名", "保存别名", "Save alias"),
    ("正在保存别名…", "正在保存别名…", "Saving alias…"),
    ("远程主机", "远程主机", "Remote host"),
    ("本机连接", "本机连接", "Local connection"),
    ("Relay 中转", "Relay 中转", "Relay connection"),
    ("连接中", "连接中", "Connecting"),
    ("NAT 直连", "NAT 直连", "Direct connection (NAT)"),
    ("连接详情", "连接详情", "Connection details"),
    ("端到端延迟：{} 毫秒", "端到端延迟：{} 毫秒", "End-to-end latency: {} ms"),
    ("端到端延迟：不可用", "端到端延迟：不可用", "End-to-end latency: unavailable"),
    ("远程主机名未能确认保存，下次连接时会重试。", "远程主机名未能确认保存，下次连接时会重试。", "Could not confirm the remote hostname was saved. It will be retried on the next connection."),
    ("你", "你", "You"),
    ("主机别名不能包含换行或控制字符", "主机别名不能包含换行或控制字符", "Host aliases cannot contain line breaks or control characters"),
    ("主机别名最多为 128 字节", "主机别名最多为 128 字节", "Host aliases must be at most 128 bytes"),
    ("主机别名未能确认保存，请重试；仍显示上次确认的名称。", "主机别名未能确认保存，请重试；仍显示上次确认的名称。", "Could not confirm the alias was saved. The last confirmed name is still shown; please retry."),
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
