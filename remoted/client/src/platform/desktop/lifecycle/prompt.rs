//! Main-window-owned prompts cannot be orphaned by closing a settings window.
use super::*;
use nsh_base::{button, dialog, theme};
use tokio::sync::oneshot;

#[derive(Clone, PartialEq)]
pub(super) struct Prompt {
    title: String,
    description: String,
    buttons: Vec<Choice>,
}
#[derive(Clone, PartialEq)]
struct Choice {
    id: &'static str,
    label: &'static str,
    destructive: bool,
}
pub(super) struct Answer {
    pub action: Option<String>,
}
impl Prompt {
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            buttons: Vec::new(),
        }
    }
    pub fn button(mut self, id: &'static str, label: &'static str, destructive: bool) -> Self {
        self.buttons.push(Choice {
            id,
            label,
            destructive,
        });
        self
    }
}

impl Lifecycle {
    pub(super) async fn show(self, prompt: Prompt) -> Result<Answer, ()> {
        let (sender, receiver) = oneshot::channel();
        self.answer(None);
        if let Some(settings) = self.settings {
            settings.set(false);
        }
        self.reply.replace(Some(sender));
        self.prompt.set(Some(prompt));
        receiver
            .await
            .map(|action| Answer { action })
            .map_err(|_| ())
    }
    pub(super) fn answer(self, action: Option<String>) {
        self.prompt.set(None);
        if let Some(reply) = self.reply.replace(None) {
            let _ = reply.send(action);
        }
    }
    pub(super) fn render(self) -> impl IntoElement {
        when(
            move || self.prompt.with(Option::is_some),
            move || {
                let prompt = self.prompt.get_untracked().expect("shown close prompt");
                dialog()
                    .role(AccessibilityRole::AlertDialog)
                    .label(prompt.title.clone())
                    .on_cancel(move || self.answer(None))
                    .child(
                        view()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .p_4()
                            .bg(move || {
                                theme()
                                    .color("overlay")
                                    .unwrap_or(theme().foreground)
                                    .opacity(0.4)
                            })
                            .child(
                                view()
                                    .w_full()
                                    .max_w(px(480.0))
                                    .p_5()
                                    .gap_4()
                                    .rounded(move || theme().radius)
                                    .border_1()
                                    .border_color(move || theme().border)
                                    .bg(move || theme().popover)
                                    .text_color(move || theme().popover_foreground)
                                    .child(text(prompt.title).font_weight(FontWeight::SEMIBOLD))
                                    .child(text(prompt.description).text_sm())
                                    .child(view().flex_row().justify_end().gap_2().children(
                                        prompt.buttons.into_iter().map(move |choice| {
                                            button()
                                                .accessibility_label(choice.label)
                                                .on_press(move || {
                                                    self.answer(Some(choice.id.into()))
                                                })
                                                .min_h(px(36.0))
                                                .px_3()
                                                .py_2()
                                                .rounded(move || theme().radius)
                                                .bg(move || {
                                                    if choice.destructive {
                                                        theme().destructive
                                                    } else {
                                                        theme().secondary
                                                    }
                                                })
                                                .text_color(move || {
                                                    if choice.destructive {
                                                        theme()
                                                            .color("destructive-foreground")
                                                            .unwrap_or(theme().primary_foreground)
                                                    } else {
                                                        theme().secondary_foreground
                                                    }
                                                })
                                                .hover(|s| s.opacity(0.85))
                                                .focus(|s| {
                                                    s.outline(px(2.0))
                                                        .outline_color(move || theme().ring)
                                                })
                                                .child(text(choice.label))
                                        }),
                                    )),
                            ),
                    )
            },
        )
    }
}
