//! Presentation state only; transport and control ownership stay in the controller.
use neko_remoted_protocol::messages::SessionBackend;
use nsh_core::{platform::Os, prelude::*};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Herdr,
    Terminal,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Presentation {
    composer: bool,
}
impl Default for Presentation {
    fn default() -> Self {
        Self {
            composer: os() == Os::Android,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Navigation {
    pub section: Signal<Section>,
    /// Explicit ordinary-terminal choice, independent of the saved startup type.
    pub ordinary_backend: Signal<SessionBackend>,
    pub drawer: Signal<bool>,
    pub sidebar: Signal<bool>,
    pub settings: Signal<bool>,
    pub files: Signal<bool>,
    /// The grant is part of the workspace identity; different hosts may reuse IDs.
    pub workspace: Signal<(String, String)>,
    presentations: Signal<HashMap<String, Presentation>>,
}
impl Navigation {
    /// Desktop settings have their own native window and never replace the terminal.
    pub fn inline_settings(self) -> bool {
        os() == Os::Android && self.settings.get()
    }
    pub fn new(setup: bool) -> Self {
        Self {
            section: signal(if os() == Os::Android {
                Section::Herdr
            } else {
                Section::Terminal
            }),
            ordinary_backend: signal(SessionBackend::Native),
            drawer: signal(false),
            sidebar: signal(true),
            settings: signal(setup),
            files: signal(false),
            workspace: signal((String::new(), String::new())),
            presentations: signal(HashMap::new()),
        }
    }
    pub fn select_initial_backend(self, backend: SessionBackend) {
        match backend {
            SessionBackend::Herdr => self.section.set(Section::Herdr),
            SessionBackend::Native | SessionBackend::Hc => {
                self.ordinary_backend.set(backend);
                self.section.set(Section::Terminal);
            }
        }
    }
    pub fn composer_visible(self, stream: &str) -> bool {
        self.presentations
            .with(|values| values.get(stream).copied().unwrap_or_default().composer)
    }
    pub fn toggle_composer(self, stream: &str) {
        self.presentations.update(|values| {
            let value = values.entry(stream.into()).or_default();
            value.composer = !value.composer;
        });
    }
    pub fn retain_sessions(self, streams: &[String]) {
        if self
            .presentations
            .with_untracked(|values| values.keys().any(|key| !streams.contains(key)))
        {
            self.presentations
                .update(|values| values.retain(|key, _| streams.contains(key)));
        }
    }
    pub fn show_session(self) {
        self.drawer.set(false);
        self.settings.set(false);
        self.files.set(false);
    }
}
pub fn wide() -> bool {
    window().width.0 >= 760.0
}

/// Apply the acknowledged default at entry, never in a creation handler or on save.
#[cfg(not(target_os = "android"))]
pub(super) fn install_startup_selection(ui: super::state::Ui, preferences: crate::preferences::Ui) {
    let local = computed(move || ui.model.with(|model| model.local));
    let previous = stored(None::<bool>);
    effect(move || {
        let local = local.get();
        if let Some(backend) = preferences.startup_backend() {
            let enter = previous.with(|previous| {
                let enter = previous.is_none() || (local && *previous == Some(false));
                *previous = Some(local);
                enter
            });
            if enter {
                ui.navigation.select_initial_backend(backend);
            }
        }
    });
}

#[cfg(all(test, not(target_os = "android")))]
#[path = "navigation_tests.rs"]
mod tests;
