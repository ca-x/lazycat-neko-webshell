use crate::preferences::tr;
use nsh_base::theme;
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Devices,
    Back,
    Herdr,
    Terminal,
    Settings,
    More,
    Files,
    Control,
    Add,
    Refresh,
    Monitor,
    Close,
}

impl Icon {
    fn markup(self) -> &'static str {
        match self {
            Self::Devices => include_str!("icons/PanelLeft.svg"),
            Self::Back => include_str!("icons/ChevronLeft.svg"),
            Self::Herdr => include_str!("icons/herdr.svg"),
            Self::Terminal => include_str!("icons/TerminalSquare.svg"),
            Self::Settings => include_str!("icons/Settings.svg"),
            Self::More => include_str!("icons/Ellipsis.svg"),
            Self::Files => include_str!("icons/Files.svg"),
            Self::Control => include_str!("icons/Unlock.svg"),
            Self::Add => include_str!("icons/Plus.svg"),
            Self::Refresh => include_str!("icons/RefreshCw.svg"),
            Self::Monitor => include_str!("icons/Monitor.svg"),
            Self::Close => include_str!("icons/X.svg"),
        }
    }
}

pub fn render(icon: Icon) -> impl IntoElement {
    mark(icon.markup())
}

fn mark(markup: &'static str) -> impl IntoElement {
    svg(ImageSrc::svg_markup(markup))
        .w(px(18.0))
        .h(px(18.0))
        .flex_shrink(0.0)
        .text_color(move || theme().foreground)
}

fn agent_markup(kind: &str) -> &'static str {
    let key = kind
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '-' | '_'))
        .collect::<String>();
    match key.as_str() {
        "pi" | "omp" => include_str!("icons/agents/pi.svg"),
        "qwen" => include_str!("icons/agents/qwen.svg"),
        "goose" => include_str!("icons/agents/goose.svg"),
        "claude" | "claudecode" | "anthropic" => include_str!("icons/agents/claudecode.svg"),
        "codex" => include_str!("icons/agents/codex.svg"),
        "gemini" | "geminicli" => include_str!("icons/agents/geminicli.svg"),
        "cursor" => include_str!("icons/agents/cursor.svg"),
        "devin" => include_str!("icons/agents/devin.svg"),
        "cline" => include_str!("icons/agents/cline.svg"),
        "mastracode" => include_str!("icons/agents/mastra.svg"),
        "agy" | "antigravity" | "antigravitycli" => include_str!("icons/agents/antigravity.svg"),
        "opencode" => include_str!("icons/agents/opencode.svg"),
        "copilot" | "githubcopilot" => include_str!("icons/agents/copilot.svg"),
        "kimi" => include_str!("icons/agents/kimi.svg"),
        "kiro" => include_str!("icons/agents/kiro.svg"),
        "amp" => include_str!("icons/agents/amp.svg"),
        "grok" | "grokbuild" => include_str!("icons/agents/grok.svg"),
        "hermes" | "hermesagent" => include_str!("icons/agents/hermesagent.svg"),
        "kilo" | "kilocode" => include_str!("icons/agents/kilocode.svg"),
        "qoder" | "qodercli" => include_str!("icons/agents/qoder.svg"),
        _ => Icon::Terminal.markup(),
    }
}

pub fn agent_with_status(kind: &str, status: Option<&str>) -> impl IntoElement + use<> {
    let markup = agent_markup(kind);
    let (_, _, token) = status_info(status);
    svg(ImageSrc::svg_markup(markup))
        .w(px(18.0))
        .h(px(18.0))
        .flex_shrink(0.0)
        .text_color(move || activity_color(token))
}

fn status_info(status: Option<&str>) -> (&'static str, &'static str, &'static str) {
    match status {
        Some("working") => ("◐", "工作中", "agent-working"),
        Some("blocked") => ("!", "需要处理", "agent-blocked"),
        Some("error") => ("!", "错误", "agent-blocked"),
        Some("done") => ("✓", "已完成", "agent-done"),
        Some("idle") => ("○", "空闲", "agent-idle"),
        _ => ("·", "状态未知", "agent-unknown"),
    }
}

fn activity_color(token: &'static str) -> nsh_core::element::AdaptiveColor {
    theme().color(token).unwrap_or_else(|| match token {
        "agent-blocked" => theme().destructive,
        "agent-working" | "agent-done" => theme().primary,
        _ => theme().muted_foreground,
    })
}

pub fn activity(status: Option<&str>) -> impl IntoElement + use<> {
    let (glyph, label, token) = status_info(status);
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .flex_shrink(0.0)
        .accessible(true)
        .label(move || tr(label))
        .child(
            text(glyph)
                .text_sm()
                .text_color(move || activity_color(token)),
        )
        .tooltip(super::tooltip(label))
}

pub fn program(
    record: &neko_remoted_protocol::messages::ProgramRecord,
) -> impl IntoElement + use<> {
    use neko_remoted_protocol::messages::ProgramState;
    let status = match record.state {
        ProgramState::Unknown => None,
        ProgramState::Idle => Some("idle"),
        ProgramState::Working => Some("working"),
        ProgramState::Done => Some("done"),
        ProgramState::Blocked => Some("blocked"),
        ProgramState::Error => Some("error"),
    };
    let app = record.app.clone();
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(agent_with_status(
            record.app.as_deref().unwrap_or_default(),
            status,
        ))
        .child(
            text(move || {
                app.clone()
                    .map(SharedString::from)
                    .unwrap_or_else(|| tr("终端程序"))
            })
            .text_xs(),
        )
        .child(activity(status))
}

pub fn primary_program(
    records: &[neko_remoted_protocol::messages::ProgramRecord],
) -> Option<&neko_remoted_protocol::messages::ProgramRecord> {
    use neko_remoted_protocol::messages::ProgramState;
    let priority = |state| match state {
        ProgramState::Error => 5,
        ProgramState::Blocked => 4,
        ProgramState::Working => 3,
        ProgramState::Done => 2,
        ProgramState::Idle => 1,
        ProgramState::Unknown => 0,
    };
    records.iter().reduce(|selected, candidate| {
        if priority(candidate.state) > priority(selected.state) {
            candidate
        } else {
            selected
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use neko_remoted_protocol::messages::{ProgramRecord, ProgramState};

    #[test]
    fn webshell_aliases_resolve_without_guessing_from_labels() {
        for (alias, canonical) in [
            ("omp", "pi"),
            ("anthropic", "claude"),
            ("claude-code", "claude"),
            ("gemini_cli", "gemini"),
            ("github copilot", "copilot"),
            ("antigravitycli", "agy"),
            ("hermesagent", "hermes"),
            ("kilocode", "kilo"),
            ("qoder", "qodercli"),
            ("grokbuild", "grok"),
            ("  Claude\tCode\n", "claude"),
        ] {
            assert_eq!(agent_markup(alias), agent_markup(canonical), "{alias}");
            assert_ne!(agent_markup(alias), Icon::Terminal.markup());
        }
        for unknown in ["mastra", "My Claude project", "Shell", ""] {
            assert_eq!(agent_markup(unknown), Icon::Terminal.markup());
        }
    }

    #[test]
    fn status_tokens_and_program_priority_follow_webshell() {
        for (status, glyph, token) in [
            ("working", "◐", "agent-working"),
            ("blocked", "!", "agent-blocked"),
            ("error", "!", "agent-blocked"),
            ("done", "✓", "agent-done"),
            ("idle", "○", "agent-idle"),
            ("unknown", "·", "agent-unknown"),
        ] {
            let actual = status_info(Some(status));
            assert_eq!((actual.0, actual.2), (glyph, token));
        }
        assert_eq!(status_info(Some("invented")), status_info(None));
        let records = [
            ProgramState::Unknown,
            ProgramState::Idle,
            ProgramState::Done,
            ProgramState::Working,
            ProgramState::Blocked,
            ProgramState::Error,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, state)| ProgramRecord {
            id: index.to_string(),
            state,
            kind: None,
            progress: None,
            app: None,
            title: Some("Claude project".into()),
            msg: None,
        })
        .collect::<Vec<_>>();
        assert!(primary_program(&[]).is_none());
        for length in 1..=records.len() {
            assert_eq!(
                primary_program(&records[..length]).unwrap().id,
                records[length - 1].id
            );
        }
        let mut tied = vec![records[5].clone(), records[5].clone()];
        tied[1].id = "second".into();
        assert_eq!(primary_program(&tied).unwrap().id, records[5].id);
    }
}
