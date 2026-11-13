//! Reactive preference state with acknowledged, serialized protected-storage writes.

use super::{Preferences, catalog::Catalog, io, locale};
use anyhow::Result;
use neko_remoted_protocol::messages::SessionBackend;
use nsh_core::element::Owner;
use nsh_core::prelude::*;

/// Loading and saving feedback; failures retain the last acknowledged settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveState {
    /// Storage is being read; editing is disabled.
    Loading,
    /// Preferences are loaded and no write is pending.
    Ready,
    /// One write is pending; additional changes are disabled until acknowledgment.
    Saving,
    /// Storage could not be loaded; retry before editing to avoid overwriting it.
    LoadFailed(String),
    /// A write failed; the previous preference values remain active.
    SaveFailed(String),
}

/// App-lifetime state shared by the preference view, terminals, and translated labels.
#[derive(Clone, Copy, Debug)]
pub struct Ui {
    values: Signal<Preferences>,
    state: Signal<SaveState>,
    catalog: &'static Catalog,
    owner: Owner,
}

impl Ui {
    /// Start once in the app owner, then pass this handle to every preference consumer.
    ///
    /// The bundled catalog is validated immediately. Storage failures are reported
    /// through [`Self::state`] and never trigger a write of default preferences.
    pub fn start() -> Result<Self> {
        let catalog = Catalog::shared()?;
        let ui = Self::from_loaded(Preferences::defaults(catalog))?;
        ui.state.set(SaveState::Loading);
        ui.read_storage();
        Ok(ui)
    }

    /// Install loaded settings synchronously in the app owner before composing child views.
    ///
    /// Also provides the context consumed by `preferences::tr`. Start only once per
    /// app owner; do not initialize another handle each time settings opens.
    pub fn from_loaded(preferences: Preferences) -> Result<Self> {
        let catalog = Catalog::shared()?;
        preferences.validate(catalog)?;
        let ui = Self {
            values: signal(preferences),
            state: signal(SaveState::Ready),
            catalog,
            owner: Owner::current(),
        };
        ui.apply();
        provide_context(ui);
        Ok(ui)
    }

    /// Current acknowledged preferences; reactive when read in a binding.
    pub fn preferences(self) -> Preferences {
        self.values.get()
    }

    /// Initial local terminal type, from the last acknowledged document.
    ///
    /// Initial selection is unavailable until loading succeeds. A pending or failed save
    /// retains the previous backend, just like the other active preferences.
    pub fn startup_backend(self) -> Option<SessionBackend> {
        match self.state.get() {
            SaveState::Loading | SaveState::LoadFailed(_) => None,
            SaveState::Ready | SaveState::Saving | SaveState::SaveFailed(_) => {
                Some(self.values.with(|preferences| preferences.default_backend))
            }
        }
    }

    /// Current loading, saving, or failure state; reactive in a binding.
    pub fn state(self) -> SaveState {
        self.state.get()
    }

    /// Whether a change can be accepted without racing a pending storage operation.
    pub fn editable(self) -> bool {
        matches!(
            self.state.get(),
            SaveState::Ready | SaveState::SaveFailed(_)
        )
    }

    /// The shared catalog, including license texts and WebShell theme IDs.
    pub fn catalog(self) -> &'static Catalog {
        self.catalog
    }

    /// The selected terminal theme; reactive when called while rendering a line.
    pub fn theme(self) -> &'static super::catalog::TerminalTheme {
        self.values.with(|preferences| {
            self.catalog
                .find(&preferences.theme_id)
                .unwrap_or(&self.catalog.themes[0])
        })
    }

    /// Translate one key using the current language and reactive platform locale.
    pub fn tr(self, key: &str) -> &str {
        locale::tr(
            self.values.with(|preferences| preferences.language),
            locale().as_str(),
            key,
        )
    }

    /// Validate and persist one change; apply only after protected storage acknowledges it.
    ///
    /// Calls while loading or saving are ignored; controls should use [`Self::editable`].
    /// Failed loads remain locked until [`Self::retry_load`] succeeds. A failed save
    /// keeps the old document active and allows the user to try the change again.
    pub fn update(self, change: impl FnOnce(&mut Preferences)) {
        if !untracked(|| self.editable()) {
            return;
        }
        let mut next = self.values.get_untracked();
        change(&mut next);
        if next == self.values.get_untracked() {
            return;
        }
        let encoded = match next.encode(self.catalog) {
            Ok(encoded) => encoded,
            Err(error) => {
                self.state.set(SaveState::SaveFailed(error.to_string()));
                return;
            }
        };
        self.state.set(SaveState::Saving);
        self.owner.run(|| {
            spawn(async move {
                match io::write(encoded).await {
                    Ok(()) => {
                        self.values.set(next);
                        self.apply();
                        self.state.set(SaveState::Ready);
                    }
                    Err(error) => self.state.set(SaveState::SaveFailed(error.to_string())),
                }
            })
        });
    }

    /// Retry a failed read without permitting writes over unread or corrupt storage.
    pub fn retry_load(self) {
        if matches!(self.state.get_untracked(), SaveState::LoadFailed(_)) {
            self.state.set(SaveState::Loading);
            self.read_storage();
        }
    }

    fn read_storage(self) {
        self.owner.run(|| {
            spawn(async move {
                let result = super::load().await;
                match result {
                    Ok(preferences) => {
                        self.values.set(preferences);
                        self.apply();
                        self.state.set(SaveState::Ready);
                    }
                    Err(error) => self.state.set(SaveState::LoadFailed(error.to_string())),
                }
            })
        });
    }

    fn apply(self) {
        let preferences = self.values.get_untracked();
        set_color_scheme(preferences.appearance.native());
        set_locale(preferences.language.native());
    }
}
