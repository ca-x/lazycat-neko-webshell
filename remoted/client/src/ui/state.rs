use super::terminal_surface::Surface;
use crate::controller::{self, Client, Command, Model, Update};
use nsh_core::prelude::*;
use std::collections::{HashMap, VecDeque};

pub struct Chunk {
    pub text: String,
    pub reset: bool,
}

#[derive(Default)]
struct CachedWorkspace {
    identity: Option<(bool, Option<crate::storage::Binding>)>,
    generation: u64,
    refs: HashMap<String, Surface>,
    active: String,
    section: Option<super::navigation::Section>,
    space: (String, String),
    presentations: Vec<(String, bool)>,
}

#[derive(Clone, Copy)]
pub struct Ui {
    pub model: Signal<Model>,
    pub active: Signal<String>,
    pub navigation: super::navigation::Navigation,
    client: Stored<Client>,
    pub refs: Stored<HashMap<String, Surface>>,
    pub pending: Stored<HashMap<String, VecDeque<Chunk>>>,
    owner: Owner,
    workspace: Signal<u64>,
    workspaces: Stored<HashMap<u64, CachedWorkspace>>,
    dismissed: Signal<std::collections::HashSet<(u64, String)>>,
}

impl Ui {
    pub fn start() -> Self {
        let (client, mut receiver) = controller::start();
        let ui = Self::new(Model::default(), client);
        spawn(async move {
            while let Some(update) = receiver.recv().await {
                match update {
                    Update::WorkspaceModel {
                        workspace,
                        selected,
                        model,
                    } => {
                        ui.workspace_model(workspace, selected, *model);
                    }
                    Update::WorkspaceOutput {
                        workspace,
                        generation,
                        stream,
                        text,
                        reset,
                    } => {
                        // Each worker publishes TerminalAttached before receiving output, and
                        // its ordered update channel creates the cached VT before this event.
                        let surface = ui.workspaces.with(|workspaces| {
                            workspaces
                                .get(&workspace)
                                .filter(|cached| cached.generation == generation)
                                .and_then(|cached| cached.refs.get(&stream).cloned())
                        });
                        if let Some(surface) = surface {
                            if reset {
                                surface.reset();
                            }
                            surface.write(text);
                        }
                    }
                    Update::Model(model) => ui.apply_model(model),
                    Update::Output {
                        generation,
                        stream,
                        text,
                        reset,
                    } => {
                        if generation != ui.model.with_untracked(|model| model.generation) {
                            continue;
                        }
                        let api = ui.refs.with(|refs| refs.get(&stream).cloned());
                        if let Some(api) = api {
                            ui.flush(&stream, &api);
                            if reset {
                                api.reset();
                            }
                            api.write(text);
                        } else {
                            let exceeded = ui.pending.with(|pending| {
                                let queue = pending.entry(stream).or_default();
                                if reset {
                                    queue.clear();
                                }
                                let count: usize = queue.iter().map(|chunk| chunk.text.len()).sum();
                                if count + text.len() > 1024 * 1024 || queue.len() >= 256 {
                                    return true;
                                }
                                queue.push_back(Chunk { text, reset });
                                false
                            });
                            if exceeded {
                                ui.send(Command::Suspend);
                                ui.error("终端显示缓冲已满，已断开以保留输出顺序".into());
                            }
                        }
                    }
                }
            }
        });
        spawn(async move {
            loop {
                timer(std::time::Duration::from_millis(250)).await;
                for event in crate::platform::drain_events() {
                    if event["lifecycle"] == "destroyed" {
                        ui.send(Command::Suspend);
                    }
                }
            }
        });
        ui
    }

    pub(super) fn new(model: Model, client: Client) -> Self {
        let active = model
            .panes
            .last()
            .map(|pane| pane.stream.clone())
            .unwrap_or_default();
        let navigation =
            super::navigation::Navigation::new(!model.ready() && model.bindings.is_empty());
        if let Some(pane) = model.panes.last() {
            navigation.section.set(match pane.kind {
                controller::PaneKind::Herdr => super::navigation::Section::Herdr,
                controller::PaneKind::Terminal => super::navigation::Section::Terminal,
            });
        }
        Self {
            model: signal_with(model, SignalEquals::Never),
            active: signal(active),
            navigation,
            client: stored(client),
            refs: stored(HashMap::new()),
            pending: stored(HashMap::new()),
            owner: Owner::current(),
            workspace: signal(0),
            workspaces: stored(HashMap::new()),
            dismissed: signal(std::collections::HashSet::new()),
        }
    }

    fn apply_model(self, model: Box<Model>) {
        let newly_attached = model.panes.iter().rev().find(|pane| {
            !self.model.with_untracked(|previous| {
                previous.panes.iter().any(|old| old.stream == pane.stream)
            })
        });
        if let Some(pane) = newly_attached {
            self.navigation.section.set(match pane.kind {
                controller::PaneKind::Herdr => super::navigation::Section::Herdr,
                controller::PaneKind::Terminal => super::navigation::Section::Terminal,
            });
            self.active.set(pane.stream.clone());
            self.navigation.show_session();
        } else if !self.active.get_untracked().is_empty()
            && !model
                .panes
                .iter()
                .any(|pane| pane.stream == self.active.get_untracked())
        {
            let next = self.model.with_untracked(|previous| {
                let index = previous
                    .panes
                    .iter()
                    .position(|pane| pane.stream == self.active.get_untracked())
                    .unwrap_or(0);
                model
                    .panes
                    .get(index.min(model.panes.len().saturating_sub(1)))
                    .map(|pane| pane.stream.clone())
                    .unwrap_or_default()
            });
            self.active.set(next);
        }
        self.refs.with(|refs| {
            refs.retain(|stream, _| model.panes.iter().any(|pane| &pane.stream == stream))
        });
        self.pending.with(|pending| {
            pending.retain(|stream, _| model.panes.iter().any(|pane| &pane.stream == stream))
        });
        self.model.set(*model);
    }

    fn workspace_model(self, workspace: u64, selected: Option<u64>, mut model: Model) {
        // VT buffers belong to the application owner, not to the currently mounted page.
        let identity = (model.local, model.active_binding.clone());
        let identity_changed = self.owner.run(|| {
            self.workspaces.with(|workspaces| {
                let cached = workspaces.entry(workspace).or_default();
                let identity_changed = cached.identity.as_ref() != Some(&identity);
                if identity_changed {
                    *cached = CachedWorkspace {
                        identity: Some(identity),
                        ..CachedWorkspace::default()
                    };
                }
                cached.generation = model.generation;
                cached
                    .refs
                    .retain(|stream, _| model.panes.iter().any(|pane| pane.stream == *stream));
                for pane in &model.panes {
                    cached.refs.entry(pane.stream.clone()).or_insert_with(|| {
                        Surface::for_screen(pane.kind == controller::PaneKind::Herdr)
                    });
                }
                identity_changed
            })
        });
        if identity_changed {
            self.dismissed
                .update(|dismissed| dismissed.retain(|(owner, _)| *owner != workspace));
        }
        let Some(generation) = selected else {
            return;
        };
        let switching = self.workspace.get_untracked() != workspace || identity_changed;
        if self.workspace.get_untracked() != workspace {
            self.workspaces.with(|workspaces| {
                let previous = workspaces
                    .entry(self.workspace.get_untracked())
                    .or_default();
                previous.active = self.active.get_untracked();
                previous.section = Some(self.navigation.section.get_untracked());
                previous.space = self.navigation.workspace.get_untracked();
                previous.presentations = self.navigation.presentations();
            });
        }
        self.workspace.set(workspace);
        let (refs, active, section, space, presentations) = self.workspaces.with(|workspaces| {
            let cached = &workspaces[&workspace];
            (
                cached.refs.clone(),
                cached.active.clone(),
                cached.section,
                cached.space.clone(),
                cached.presentations.clone(),
            )
        });
        self.refs.with(|current| *current = refs);
        if switching {
            self.pending.with(|pending| pending.clear());
            self.active.set(active);
            if let Some(section) = section {
                self.navigation.section.set(section);
            }
            self.navigation.workspace.set(space.clone());
        }
        model.generation = generation;
        let remembered = switching
            .then(|| self.active.get_untracked())
            .filter(|active| model.panes.iter().any(|pane| pane.stream == *active));
        self.apply_model(Box::new(model));
        if switching
            && self.active.get_untracked().is_empty()
            && let Some(pane) = self
                .model
                .with_untracked(|model| model.panes.last().cloned())
        {
            self.active.set(pane.stream);
            self.navigation.section.set(match pane.kind {
                controller::PaneKind::Herdr => super::navigation::Section::Herdr,
                controller::PaneKind::Terminal => super::navigation::Section::Terminal,
            });
        }
        if let Some(active) = remembered {
            self.active.set(active);
            if let Some(section) = section {
                self.navigation.section.set(section);
            }
            self.navigation.workspace.set(space);
        }
        if switching {
            self.navigation.restore_presentations(presentations);
        }
    }

    pub fn tab_dismissed(self, key: &str) -> bool {
        self.dismissed
            .with(|dismissed| dismissed.contains(&(self.workspace.get(), key.to_owned())))
    }
    pub fn dismiss_tab(self, key: &str) {
        self.dismissed.update(|dismissed| {
            dismissed.insert((self.workspace.get_untracked(), key.to_owned()));
        });
    }
    pub fn reopen_tab(self, key: &str) {
        self.dismissed.update(|dismissed| {
            dismissed.remove(&(self.workspace.get_untracked(), key.to_owned()));
        });
    }

    pub fn surface(self, stream: &str, snapshot: bool) -> Surface {
        if let Some(surface) = self.refs.with(|refs| refs.get(stream).cloned()) {
            return surface;
        }
        let surface = self.owner.run(|| Surface::for_screen(snapshot));
        self.refs.with(|refs| {
            refs.insert(stream.to_owned(), surface.clone());
        });
        surface
    }

    pub fn back(self) {
        if self.navigation.drawer.get_untracked() {
            self.navigation.drawer.set(false);
        } else if self.navigation.files.get_untracked() {
            self.navigation.files.set(false);
        } else if self.navigation.settings.get_untracked() {
            self.navigation.settings.set(false);
        } else {
            self.active.set(String::new());
        }
    }

    pub fn secondary(self) -> bool {
        self.navigation.drawer.get()
            || self.navigation.files.get()
            || self.navigation.settings.get()
            || !self.active.get().is_empty()
    }

    /// Explicit app-level intent, including when a connection transition has
    /// advanced before its next model update reaches the UI.
    pub fn disconnect(self) -> bool {
        match self.client.with(|client| client.send(Command::Disconnect)) {
            Ok(()) => true,
            Err(error) => {
                self.error(error.to_string());
                false
            }
        }
    }

    pub fn send(self, command: Command) {
        self.submit(command);
    }

    pub fn submit(self, command: Command) -> bool {
        match self
            .client
            .with(|client| client.send_for(self.model.with_untracked(|m| m.generation), command))
        {
            Ok(()) => true,
            Err(error) => {
                self.error(error.to_string());
                false
            }
        }
    }

    pub fn error(self, message: String) {
        self.model.update(|model| model.error = message);
    }

    pub fn flush(self, stream: &str, api: &Surface) {
        if let Some(queue) = self.pending.with(|pending| pending.remove(stream)) {
            for chunk in queue {
                if chunk.reset {
                    api.reset();
                }
                api.write(chunk.text);
            }
        }
    }
}

#[cfg(test)]
mod tests;
