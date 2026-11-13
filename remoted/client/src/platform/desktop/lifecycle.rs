//! Desktop close policy and tray actions; hiding never disposes terminal owners.

#[cfg(target_os = "linux")]
mod linux;

use crate::preferences::{CloseBehavior, Language, SaveState, Ui};
use nsh_base::{AlertDialogOptions, alert_dialog};
use nsh_core::{
    desktop::WindowAttributes,
    element::{ImageSrc, desktop},
    prelude::*,
};
use std::{future::Future, time::Duration};

/// Install once in the root owner after preferences have been initialized.
pub(crate) fn start_desktop_lifecycle(preferences: Ui) {
    let lifecycle = Lifecycle::new();
    let mut attributes = WindowAttributes::default();
    attributes.close_on_press = Some(false);
    desktop::main_window().set(attributes);
    desktop::main_window().on_close_press(move |_| {
        lifecycle.close(
            preferences.preferences().close_behavior,
            Labels::new(preferences.preferences().language),
            move |choice| remember(preferences, choice),
            tray_available,
        );
    });
    effect(move || lifecycle.install_tray(Labels::new(preferences.preferences().language)));
}

async fn remember(preferences: Ui, choice: CloseBehavior) -> Result<(), String> {
    if !preferences.editable() {
        return Err(preferences.tr("preferences.loading").to_owned());
    }
    preferences.update(|next| next.close_behavior = choice);
    // Ui publishes only acknowledged documents. Do not quit or hide before this save.
    for _ in 0..500 {
        match preferences.state() {
            SaveState::Ready if preferences.preferences().close_behavior == choice => return Ok(()),
            SaveState::SaveFailed(error) | SaveState::LoadFailed(error) => return Err(error),
            SaveState::Saving => timer(Duration::from_millis(20)).await,
            _ => break,
        }
    }
    Err(preferences.tr("preferences.save_failed").to_owned())
}

#[derive(Clone, Copy)]
struct Lifecycle {
    owner: Owner,
    generation: Signal<u64>,
    busy: Signal<bool>,
    hidden: Signal<bool>,
}

impl Lifecycle {
    fn new() -> Self {
        Self {
            owner: Owner::current(),
            generation: signal(0),
            busy: signal(false),
            hidden: signal(false),
        }
    }

    fn install_tray(self, labels: Labels) {
        desktop::set_tray(
            desktop::tray()
                .icon(ImageSrc::svg_markup(include_str!(
                    "../../ui/icons/TerminalSquare.svg"
                )))
                .tooltip("neoko-terminal")
                .on_click(move |_| self.restore())
                .menu([
                    desktop::MenuItem::new(labels.restore).on_select(move || self.restore()),
                    desktop::MenuItem::separator(),
                    desktop::MenuItem::new(labels.quit).on_select(move || self.quit()),
                ]),
        );
    }

    fn restore(self) {
        self.generation.update(|generation| *generation += 1);
        self.busy.set(false);
        self.hidden.set(false);
        let window = desktop::main_window();
        window.show();
        window.restore();
        window.focus();
    }

    fn quit(self) {
        self.generation.update(|generation| *generation += 1);
        desktop::quit();
    }

    fn close<Save, Probe>(
        self,
        behavior: CloseBehavior,
        labels: Labels,
        save: impl FnOnce(CloseBehavior) -> Save + 'static,
        probe: impl Fn() -> Probe + Copy + 'static,
    ) where
        Save: Future<Output = Result<(), String>> + 'static,
        Probe: Future<Output = bool> + 'static,
    {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.generation.update(|generation| *generation += 1);
        let generation = self.generation.get_untracked();
        self.owner.run(|| {
            spawn(async move {
                let available = behavior != CloseBehavior::Quit && probe().await;
                if self.generation.get_untracked() != generation {
                    return;
                }
                let choice = if behavior == CloseBehavior::Ask {
                    let answer = labels.choice(available).show().await;
                    if self.generation.get_untracked() != generation {
                        return;
                    }
                    let choice = match answer.ok().and_then(|answer| answer.action) {
                        Some(action) if action == "quit" => CloseBehavior::Quit,
                        Some(action) if action == "tray" => CloseBehavior::Tray,
                        _ => {
                            self.busy.set(false);
                            return;
                        }
                    };
                    if let Err(error) = save(choice).await {
                        if self.generation.get_untracked() == generation {
                            let _ = alert_dialog()
                                .title(labels.save_failed)
                                .description(error)
                                .cancel("ok", labels.ok)
                                .show()
                                .await;
                            if self.generation.get_untracked() == generation {
                                self.busy.set(false);
                            }
                        }
                        return;
                    }
                    choice
                } else {
                    behavior
                };
                if self.generation.get_untracked() != generation {
                    return;
                }
                match choice {
                    CloseBehavior::Quit => self.quit(),
                    CloseBehavior::Tray => {
                        // The panel may have gone away while the first decision or
                        // its acknowledged save was pending. Revalidate before hiding.
                        let current_available = if available && behavior == CloseBehavior::Ask {
                            probe().await
                        } else {
                            available
                        };
                        if self.generation.get_untracked() != generation {
                            return;
                        }
                        // The first decision already explains this actual fallback. A saved
                        // Tray policy needs the same feedback when no tray is available.
                        if !current_available && (behavior != CloseBehavior::Ask || available) {
                            let answer = alert_dialog()
                                .title(labels.fallback_title)
                                .description(labels.fallback)
                                .cancel("cancel", labels.cancel)
                                .action("minimize", labels.minimize)
                                .close_on_escape(true)
                                .show()
                                .await;
                            if self.generation.get_untracked() != generation {
                                return;
                            }
                            if !answer.is_ok_and(|a| a.action.as_deref() == Some("minimize")) {
                                self.busy.set(false);
                                return;
                            }
                        }
                        if current_available {
                            self.hidden.set(true);
                            desktop::main_window().hide();
                            self.watch_tray(generation, labels, probe);
                        } else {
                            desktop::main_window().minimize();
                        }
                    }
                    CloseBehavior::Ask => {}
                }
                self.busy.set(false);
            })
        });
    }

    fn watch_tray<Probe>(self, generation: u64, labels: Labels, probe: impl Fn() -> Probe + 'static)
    where
        Probe: Future<Output = bool> + 'static,
    {
        // A disappearing Linux panel must not leave a hidden, unreachable app.
        // Native macOS/Windows trays are maintained by the platform backend.
        if !cfg!(target_os = "linux") {
            return;
        }
        spawn(async move {
            loop {
                timer(Duration::from_secs(2)).await;
                if self.generation.get_untracked() != generation || !self.hidden.get_untracked() {
                    return;
                }
                let available = probe().await;
                if self.generation.get_untracked() != generation {
                    return;
                }
                if !available {
                    self.restore();
                    let _ = alert_dialog()
                        .title(labels.fallback_title)
                        .description(labels.restored)
                        .cancel("ok", labels.ok)
                        .show()
                        .await;
                    return;
                }
            }
        });
    }
}

async fn tray_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::available().await
    }
    #[cfg(not(target_os = "linux"))]
    {
        // NSH uses NSStatusItem and Shell_NotifyIconW respectively. This revision
        // exposes no tray registration acknowledgement on those platforms.
        cfg!(any(target_os = "macos", target_os = "windows"))
    }
}

#[derive(Clone, Copy)]
struct Labels {
    title: &'static str,
    body: &'static str,
    fallback: &'static str,
    fallback_title: &'static str,
    restored: &'static str,
    tray: &'static str,
    minimize: &'static str,
    quit: &'static str,
    cancel: &'static str,
    restore: &'static str,
    save_failed: &'static str,
    ok: &'static str,
}

impl Labels {
    fn new(language: Language) -> Self {
        let chinese = match language {
            Language::ZhCn => true,
            Language::EnUs => false,
            Language::System => locale().as_str().starts_with("zh"),
        };
        if chinese {
            Self {
                title: "关闭 neoko-terminal 窗口？",
                body: "退出会结束客户端及本机终端。最小化到托盘会保持终端和连接运行。选择会被记住，可在设置中更改；取消不会更改设置。",
                fallback: "当前无法确认可用的系统托盘。窗口将最小化到任务栏，终端和连接继续运行，可从任务栏恢复。",
                fallback_title: "系统托盘不可用",
                restored: "系统托盘已不可用，窗口已恢复，终端和连接仍在运行。",
                tray: "最小化到托盘",
                minimize: "最小化到任务栏",
                quit: "退出",
                cancel: "取消",
                restore: "显示终端",
                save_failed: "无法保存关闭方式，窗口仍保持打开",
                ok: "好",
            }
        } else {
            Self {
                title: "Close neoko-terminal?",
                body: "Quit ends the client and local terminals. Minimize to tray keeps terminals and connections running. Your choice is remembered and can be changed in Settings. Cancel leaves settings unchanged.",
                fallback: "A working system tray could not be confirmed. The window will minimize to the taskbar and keep terminals and connections running. Restore it from the taskbar.",
                fallback_title: "System tray unavailable",
                restored: "The system tray is no longer available. The window has been restored; terminals and connections are still running.",
                tray: "Minimize to tray",
                minimize: "Minimize to taskbar",
                quit: "Quit",
                cancel: "Cancel",
                restore: "Show terminal",
                save_failed: "Could not save close behavior; the window remains open",
                ok: "OK",
            }
        }
    }

    fn choice(self, available: bool) -> AlertDialogOptions {
        let description = if available {
            self.body.to_owned()
        } else {
            format!("{}\n\n{}", self.body, self.fallback)
        };
        alert_dialog()
            .title(self.title)
            .description(description)
            .cancel("cancel", self.cancel)
            .action("tray", self.tray)
            .destructive("quit", self.quit)
            .close_on_escape(true)
            .close_on_backdrop_press(false)
    }
}

#[cfg(test)]
mod tests;
