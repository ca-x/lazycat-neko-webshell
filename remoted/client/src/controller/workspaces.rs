//! Each workspace owns its transport, request queue and reconnect lifecycle.
//! Selecting a workspace changes presentation only; it never closes another one.
use super::*;

const MAX_WORKSPACES: usize = 18;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

// A dispatched secure-store job can keep running after its future is dropped.
// Never cancel a router route while it owns the storage writer: wait for its
// acknowledgement/publication, then discard only the superseded UI result.
async fn admitted(
    scope: &ConnectionScope,
    generation: u64,
    route: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    ensure!(
        scope.current() == generation,
        "工作区已切换，旧页面操作已丢弃"
    );
    let result = route.await;
    ensure!(
        scope.current() == generation,
        "工作区已切换，旧页面操作已丢弃"
    );
    result
}

#[derive(Clone, PartialEq, Eq)]
enum Target {
    Empty,
    Pairing,
    #[cfg(not(target_os = "android"))]
    Local,
    Remote(Binding),
}

impl Target {
    fn binding(&self) -> Option<&Binding> {
        match self {
            Self::Remote(binding) => Some(binding),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Shared {
    bindings: Vec<Binding>,
    aliases: storage::HostAliases,
    names: storage::HostNames,
}

impl Shared {
    fn of(model: &Model) -> Self {
        Self {
            bindings: model.bindings.clone(),
            aliases: model.aliases.clone(),
            names: model.host_names.clone(),
        }
    }

    fn apply(&self, model: &mut Model) {
        model.bindings = self.bindings.clone();
        model.aliases = self.aliases.clone();
        model.host_names = self.names.clone();
    }
}

struct Worker {
    client: Client,
    model: Model,
    // Reserve a target before its asynchronous actor has published a model.
    target: Target,
    activation: u64,
    shared: Shared,
    actor: tokio::task::JoinHandle<()>,
    forwarding: tokio::task::JoinHandle<()>,
}

struct Workspaces {
    workers: Vec<Worker>,
    active: usize,
    epoch: u64,
    activation: u64,
    stopping: bool,
    globals: Model,
    saved: storage::snapshots::Receiver,
    shared_revision: u64,
    auto_control: bool,
    updates: mpsc::Sender<Update>,
    incoming: mpsc::Sender<(usize, Update)>,
    scope: Arc<ConnectionScope>,
    #[cfg(not(target_os = "android"))]
    host: Option<neoko_terminal_host::Host>,
}

impl Workspaces {
    fn new(
        initial: &Actor,
        updates: mpsc::Sender<Update>,
        incoming: mpsc::Sender<(usize, Update)>,
        scope: Arc<ConnectionScope>,
        saved: storage::snapshots::Receiver,
    ) -> Self {
        Self {
            workers: Vec::new(),
            active: 0,
            // Startup owns only the initial UI. An intent queued while storage
            // or auto-connect was initializing must be handled before selection.
            epoch: 0,
            activation: 0,
            stopping: false,
            globals: initial.model.clone(),
            saved,
            shared_revision: 0,
            auto_control: false,
            updates,
            incoming,
            scope,
            #[cfg(not(target_os = "android"))]
            host: initial.host.clone(),
        }
    }

    fn spawn(&mut self, mut actor: Actor, receiver: mpsc::Receiver<Update>) -> usize {
        let id = self.workers.len();
        let (sender, commands) = mpsc::channel(QUEUE);
        actor.remote_auto_control = self.auto_control;
        let client = Client {
            sender,
            scope: Some(actor.scope.clone()),
        };
        let target = match actor.model.active_binding.clone() {
            #[cfg(not(target_os = "android"))]
            _ if actor.model.local => Target::Local,
            Some(binding) => Target::Remote(binding),
            None => Target::Empty,
        };
        let model = actor.model.clone();
        let incoming = self.incoming.clone();
        let forwarding = tokio::spawn(async move {
            let mut receiver = receiver;
            while let Some(update) = receiver.recv().await {
                if incoming.send((id, update)).await.is_err() {
                    break;
                }
            }
        });
        self.workers.push(Worker {
            client,
            activation: model.activation,
            shared: Shared::of(&model),
            model,
            target,
            actor: tokio::spawn(actor.run(commands)),
            forwarding,
        });
        id
    }

    fn create(&mut self) -> Result<usize> {
        if let Some(id) = self.workers.iter().position(|worker| {
            worker.target == Target::Empty
                && worker.model.phase == Phase::Offline
                && !worker.actor.is_finished()
        }) {
            return Ok(id);
        }
        ensure!(
            self.workers.len() < MAX_WORKSPACES,
            "同时打开的主机过多，请先断开不用的连接"
        );
        let (updates, receiver) = mpsc::channel(QUEUE);
        let mut actor = Actor::with_scope(updates, Arc::new(ConnectionScope::default()));
        Shared::of(&self.globals).apply(&mut actor.model);
        actor.model.relay = self.globals.relay.clone();
        #[cfg(not(target_os = "android"))]
        {
            actor.host = self.host.clone();
            actor.host_updates = self.host.as_ref().map(|host| host.subscribe());
            actor.model.host = self.globals.host.clone();
        }
        Ok(self.spawn(actor, receiver))
    }

    fn activate(&mut self, id: usize) -> Result<()> {
        let activation = self
            .activation
            .checked_add(1)
            .context("工作区同步世代已耗尽，请重新启动客户端")?;
        let worker = &mut self.workers[id];
        let permit = worker
            .client
            .sender
            .try_reserve()
            .context("工作区操作队列繁忙或已关闭，请稍后重试")?;
        // Selection acknowledgements are independent of the durable snapshot
        // revision. Actor echoes can never write shared preferences back.
        worker.shared = Shared::of(&self.globals);
        worker.activation = activation;
        self.activation = activation;
        permit.send(Command::Activate {
            epoch: activation,
            globals: Box::new(self.globals.clone()),
        });
        Ok(())
    }

    fn sync(&mut self, id: usize) -> Result<()> {
        if self.workers[id].shared != Shared::of(&self.globals) {
            self.activate(id)?;
        }
        Ok(())
    }

    async fn select(&mut self, id: usize) -> Result<()> {
        self.activate(id)?;
        self.active = id;
        self.epoch = self.scope.current();
        self.scope
            .select(self.workers[id].target.binding().cloned());
        Ok(())
    }

    fn selected(&self, id: usize, model: &Model) -> Option<u64> {
        (id == self.active
            && self.scope.current() == self.epoch
            && model.activation == self.workers[id].activation)
            .then_some(self.epoch)
    }

    fn project(&self, model: &mut Model) {
        Shared::of(&self.globals).apply(model);
        #[cfg(not(target_os = "android"))]
        {
            model.host = self.globals.host.clone();
        }
    }

    fn warm(&self, id: usize) -> bool {
        let worker = &self.workers[id];
        worker.model.ready()
            && worker
                .client
                .scope
                .as_ref()
                .is_some_and(|scope| scope.current() == worker.model.generation)
    }

    async fn command(&mut self, command: Command) -> Result<()> {
        let shared_changed = self.absorb_saved();
        let (generation, command) = match command {
            Command::Scoped {
                generation,
                command,
            } => (generation, *command),
            command => (self.scope.current(), command),
        };
        // Quit is process-wide even if a concurrent selection advanced the UI.
        if matches!(command, Command::Suspend) {
            self.stopping = true;
            return Ok(());
        }
        if shared_changed {
            self.refresh_visible().await;
        }
        let scope = self.scope.clone();
        admitted(&scope, generation, self.route(command, generation)).await
    }

    async fn route(&mut self, command: Command, generation: u64) -> Result<()> {
        match command {
            Command::Scoped { .. } | Command::Activate { .. } => {
                bail!("不接受内部或嵌套工作区命令")
            }
            Command::ConfigureTerminal {
                remote_auto_control,
            } => {
                self.auto_control = remote_auto_control;
                for worker in &self.workers {
                    worker.client.send(Command::ConfigureTerminal {
                        remote_auto_control,
                    })?;
                }
            }
            Command::Reconnect(binding) => {
                storage::validate(&binding)?;
                ensure!(
                    self.globals.bindings.contains(&binding),
                    "设备配对已变化，请重新选择"
                );
                let target = Target::Remote(binding.clone());
                let id = match self
                    .workers
                    .iter()
                    .position(|worker| worker.target == target)
                {
                    Some(id) => id,
                    None => self.create()?,
                };
                let warm = self.warm(id)
                    && self.workers[id].model.active_binding.as_ref() == Some(&binding);
                if warm {
                    storage::select_binding(&binding).await?;
                }
                // A storage acknowledgement may yield to a newer user intent.
                ensure!(
                    self.scope.current() == generation,
                    "工作区已切换，旧页面操作已丢弃"
                );
                self.select(id).await?;
                if !warm {
                    self.workers[id]
                        .client
                        .send(Command::Reconnect(binding.clone()))?;
                }
                self.workers[id].target = target;
                self.scope.select(Some(binding));
            }
            #[cfg(not(target_os = "android"))]
            Command::SelectLocal => {
                let id = match self
                    .workers
                    .iter()
                    .position(|worker| worker.target == Target::Local)
                {
                    Some(id) => id,
                    None => self.create()?,
                };
                let warm = self.warm(id) && self.workers[id].model.local;
                if warm {
                    storage::select_local_workspace().await?;
                }
                ensure!(
                    self.scope.current() == generation,
                    "工作区已切换，旧页面操作已丢弃"
                );
                self.select(id).await?;
                if !warm {
                    self.workers[id].client.send(Command::SelectLocal)?;
                }
                self.workers[id].target = Target::Local;
                self.scope.select(None);
            }
            Command::Check(address) => {
                let id = match self.workers.iter().position(|worker| {
                    worker.target == Target::Pairing
                        && matches!(
                            worker.model.phase,
                            Phase::Offline | Phase::Checking | Phase::RelayReady
                        )
                }) {
                    Some(id) => id,
                    None => self.create()?,
                };
                self.select(id).await?;
                self.workers[id].client.send(Command::Check(address))?;
                self.workers[id].target = Target::Pairing;
                self.scope.select(None);
            }
            Command::Forget(binding) => {
                ensure!(
                    self.globals.bindings.contains(&binding),
                    "设备配对已变化，请重新选择"
                );
                let owner = self
                    .workers
                    .iter()
                    .position(|worker| worker.target.binding() == Some(&binding));
                let id = owner.unwrap_or(self.active);
                if id == self.active && generation != self.epoch {
                    self.select(id).await?;
                } else {
                    self.sync(id)?;
                }
                if owner.is_some() {
                    // A reserved connection may not have published its selected
                    // binding yet. Forget must still cancel its in-flight handshake.
                    self.workers[id]
                        .client
                        .scope
                        .as_ref()
                        .unwrap()
                        .select(Some(binding.clone()));
                }
                self.workers[id].client.send(Command::Forget(binding))?;
            }
            Command::RenameBinding { binding, alias } => {
                ensure!(generation == self.epoch, "工作区已切换，旧页面操作已丢弃");
                ensure!(
                    self.globals.bindings.contains(&binding),
                    "设备配对已变化，请重新选择"
                );
                self.sync(self.active)?;
                self.workers[self.active]
                    .client
                    .send(Command::RenameBinding { binding, alias })?;
            }
            #[cfg(not(target_os = "android"))]
            Command::Host(action) => self.workers[self.active]
                .client
                .send(Command::Host(action))?,
            Command::Suspend => unreachable!("handled before routing"),
            command => {
                if command.changes_connection() {
                    self.select(self.active).await?;
                } else {
                    ensure!(generation == self.epoch, "工作区已切换，旧页面操作已丢弃");
                    self.sync(self.active)?;
                }
                self.workers[self.active].client.send(command)?;
            }
        }
        Ok(())
    }

    fn update_metadata(&mut self, previous: &Model, model: &Model) {
        if model.relay != previous.relay {
            self.globals.relay = model.relay.clone();
        }
        #[cfg(not(target_os = "android"))]
        {
            self.globals.host = self
                .host
                .as_ref()
                .map(|host| host.snapshot())
                .or_else(|| model.host.clone());
        }
    }

    // Called before admitting commands or models, and independently on watch
    // notifications. A newer complete snapshot supersedes every older one,
    // including saves whose actor model has not reached the router yet.
    fn absorb_saved(&mut self) -> bool {
        let snapshot = self.saved.borrow_and_update().clone();
        let Some(snapshot) = snapshot.filter(|snapshot| snapshot.revision > self.shared_revision)
        else {
            return false;
        };
        self.shared_revision = snapshot.revision;
        self.globals.bindings = snapshot.bindings.clone();
        self.globals.aliases = snapshot.aliases.clone();
        self.globals.host_names = snapshot.host_names.clone();
        true
    }

    async fn refresh_visible(&self) {
        let Some(worker) = self.workers.get(self.active) else {
            return;
        };
        let mut model = worker.model.clone();
        let selected = self.selected(self.active, &model);
        if selected.is_some() {
            self.project(&mut model);
            let _ = self
                .updates
                .send(Update::WorkspaceModel {
                    workspace: self.active as u64,
                    selected,
                    model: Box::new(model),
                })
                .await;
        }
    }

    async fn receive_saved(&mut self) {
        if self.absorb_saved() {
            self.refresh_visible().await;
        }
    }

    async fn update(&mut self, id: usize, update: Update) {
        let shared_changed = self.absorb_saved();
        match update {
            Update::Model(mut model) => {
                let worker = &self.workers[id];
                if model.generation < worker.model.generation
                    || model.activation < worker.model.activation
                {
                    if shared_changed {
                        self.refresh_visible().await;
                    }
                    return;
                }
                let previous = worker.model.clone();
                self.update_metadata(&previous, &model);
                let worker = &mut self.workers[id];
                if worker.target == Target::Pairing {
                    if let Some(binding) = &model.active_binding {
                        worker.target = Target::Remote(binding.clone());
                    }
                } else if worker
                    .target
                    .binding()
                    .is_some_and(|binding| !self.globals.bindings.contains(binding))
                    && model.phase == Phase::Offline
                    && model.active_binding.is_none()
                {
                    worker.target = Target::Empty;
                }
                worker.model = (*model).clone();
                let selected = self.selected(id, &model);
                if selected.is_some() {
                    self.scope
                        .select(self.workers[id].target.binding().cloned());
                }
                self.project(&mut model);
                let _ = self
                    .updates
                    .send(Update::WorkspaceModel {
                        workspace: id as u64,
                        selected,
                        model,
                    })
                    .await;
                if id != self.active && shared_changed {
                    self.refresh_visible().await;
                }
            }

            Update::Output {
                generation,
                stream,
                text,
                reset,
            } => {
                if shared_changed {
                    self.refresh_visible().await;
                }
                if generation != self.workers[id].model.generation {
                    return;
                }
                let _ = self
                    .updates
                    .send(Update::WorkspaceOutput {
                        workspace: id as u64,
                        generation,
                        stream,
                        text,
                        reset,
                    })
                    .await;
            }
            Update::WorkspaceModel { .. } | Update::WorkspaceOutput { .. } => {
                unreachable!("only the workspace router emits workspace updates")
            }
        }
    }

    async fn shutdown(&mut self) {
        let workers = std::mem::take(&mut self.workers);
        // Closing both queues unblocks publish as well as commands. Advancing
        // private scopes cancels handshakes before Actor::run does its normal
        // disconnected()/transfer cleanup. Suspend alone leaves its loop alive.
        for worker in &workers {
            let _ = worker.client.scope.as_ref().unwrap().advance();
            worker.forwarding.abort();
        }
        let deadline = tokio::time::Instant::now() + SHUTDOWN_TIMEOUT;
        for worker in workers {
            let Worker {
                client,
                mut actor,
                forwarding,
                ..
            } = worker;
            drop(client);
            let _ = forwarding.await;
            if tokio::time::timeout_at(deadline, &mut actor).await.is_err() {
                eprintln!("工作区清理超时，取消剩余任务");
                actor.abort();
                let _ = actor.await;
            }
        }
    }
}

impl Drop for Workspaces {
    fn drop(&mut self) {
        for worker in &self.workers {
            let _ = worker.client.scope.as_ref().unwrap().advance();
            worker.forwarding.abort();
            worker.actor.abort();
        }
    }
}

pub(super) async fn run(
    updates: mpsc::Sender<Update>,
    scope: Arc<ConnectionScope>,
    mut commands: mpsc::Receiver<Command>,
) {
    // Subscribe before initialize(), which can itself persist a selection.
    let saved = storage::snapshots::subscribe();
    let (sender, receiver) = mpsc::channel(QUEUE);
    let mut initial = Actor::with_scope(sender, Arc::new(ConnectionScope::default()));
    initial.initialize().await;
    let (incoming, mut events) = mpsc::channel(QUEUE);
    let mut workspaces = Workspaces::new(&initial, updates, incoming, scope, saved);
    initial.model.activation = 0;
    workspaces.spawn(initial, receiver);
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(command) => {
                    let generation = match &command {
                        Command::Scoped { generation, .. } => *generation,
                        _ => workspaces.scope.current(),
                    };
                    if let Err(error) = workspaces.command(command).await
                        && generation == workspaces.scope.current()
                    {
                        let mut model = workspaces.workers[workspaces.active].model.clone();
                        let selected = workspaces.selected(workspaces.active, &model);
                        workspaces.project(&mut model);
                        model.error = format!("{error:#}");
                        let _ = workspaces.updates.send(Update::WorkspaceModel { workspace: workspaces.active as u64, selected, model: Box::new(model) }).await;
                    }
                    if workspaces.stopping {
                        break;
                    }
                }
                None => break,
            },
            Some((id, update)) = events.recv() => workspaces.update(id, update).await,
            _ = storage::snapshots::changed(&mut workspaces.saved) => workspaces.receive_saved().await,
            _ = workspaces.updates.closed() => break,
        }
    }
    commands.close();
    workspaces.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    type FixtureParts = (
        Workspaces,
        mpsc::Receiver<Update>,
        mpsc::Receiver<(usize, Update)>,
        tokio::sync::watch::Sender<Option<Arc<storage::snapshots::Snapshot>>>,
    );

    fn fixture() -> FixtureParts {
        let (snapshots, saved) = tokio::sync::watch::channel(None);
        let (updates, visible) = mpsc::channel(128);
        let (incoming, events) = mpsc::channel(128);
        (
            Workspaces {
                workers: Vec::new(),
                active: 0,
                epoch: 0,
                activation: 0,
                stopping: false,
                globals: Model::default(),
                saved,
                shared_revision: 0,
                auto_control: false,
                updates,
                incoming,
                scope: Arc::new(ConnectionScope::default()),
                #[cfg(not(target_os = "android"))]
                host: None,
            },
            visible,
            events,
            snapshots,
        )
    }

    // Routing tests keep the command receivers in the test. No worker can open
    // a real network connection, read a keyring, or change the user's preferences.
    fn parked(router: &mut Workspaces, model: Model) -> mpsc::Receiver<Command> {
        let (sender, commands) = mpsc::channel(QUEUE);
        let scope = Arc::new(ConnectionScope::default());
        for _ in 0..model.generation {
            scope.advance().unwrap();
        }
        scope.select(model.active_binding.clone());
        let target = match model.active_binding.clone() {
            #[cfg(not(target_os = "android"))]
            _ if model.local => Target::Local,
            Some(binding) => Target::Remote(binding),
            None => Target::Empty,
        };
        router.workers.push(Worker {
            client: Client {
                sender,
                scope: Some(scope),
            },
            activation: model.activation,
            shared: Shared::of(&model),
            target,
            model,
            actor: tokio::spawn(std::future::pending()),
            forwarding: tokio::spawn(std::future::pending()),
        });
        commands
    }

    fn binding(seed: u8) -> Binding {
        Binding {
            relay: RelayAddress::parse("wss://router-test.invalid").unwrap(),
            route: protocol::encode(&[seed; 32]),
            host_key: protocol::encode(&[seed.wrapping_add(1); 32]),
            device_id: protocol::encode(&[seed.wrapping_add(2); 16]),
        }
    }

    fn labels<T: serde::de::DeserializeOwned>(pairs: &[(&Binding, &str)]) -> T {
        let records: Vec<_> = pairs.iter().map(|(binding, name)| serde_json::json!({
            "identity": { "relay": binding.relay, "route": binding.route, "host_key": binding.host_key },
            "name": name,
        })).collect();
        serde_json::from_value(serde_json::json!(records)).unwrap()
    }

    fn commit(
        snapshots: &tokio::sync::watch::Sender<Option<Arc<storage::snapshots::Snapshot>>>,
        revision: u64,
        model: &Model,
    ) {
        let _ = snapshots.send_replace(Some(Arc::new(storage::snapshots::Snapshot {
            revision,
            bindings: model.bindings.clone(),
            aliases: model.aliases.clone(),
            host_names: model.host_names.clone(),
        })));
    }

    fn ready(binding: &Binding, bindings: &[Binding]) -> Model {
        Model {
            phase: Phase::Ready,
            permission: Some(Permission::Operate),
            active_binding: Some(binding.clone()),
            bindings: bindings.to_vec(),
            ..Model::default()
        }
    }

    fn model_update(update: Option<Update>) -> (u64, Option<u64>, Box<Model>) {
        match update {
            Some(Update::WorkspaceModel {
                workspace,
                selected,
                model,
            }) => (workspace, selected, model),
            _ => panic!("expected workspace model"),
        }
    }

    #[tokio::test]
    async fn a_selection_queued_during_initialization_cannot_adopt_the_startup_model() {
        let (actor_updates, _receiver) = mpsc::channel(1);
        let mut initial = Actor::new(actor_updates);
        let a = binding(1);
        initial.model = ready(&a, std::slice::from_ref(&a));
        let (updates, mut visible) = mpsc::channel(8);
        let (incoming, _events) = mpsc::channel(8);
        let scope = Arc::new(ConnectionScope::default());
        let epoch = scope.advance().unwrap(); // User selects while initialize() awaits.
        let (_snapshots, saved) = tokio::sync::watch::channel(None);
        let mut router = Workspaces::new(&initial, updates, incoming, scope, saved);
        let mut commands = parked(&mut router, initial.model.clone());
        router
            .update(0, Update::Model(Box::new(initial.model.clone())))
            .await;
        assert_eq!(model_update(visible.recv().await).1, None);
        // Only admitting the queued intent may present a model in that epoch.
        router.select(0).await.unwrap();
        let Some(Command::Activate {
            epoch: activation, ..
        }) = commands.recv().await
        else {
            panic!("activation barrier");
        };
        initial.model.activation = activation;
        router
            .update(0, Update::Model(Box::new(initial.model)))
            .await;
        assert_eq!(model_update(visible.recv().await).1, Some(epoch));
    }

    #[tokio::test]
    async fn old_models_and_commands_cannot_enter_a_new_ui_generation() {
        let (mut router, mut visible, _events, _snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let model = ready(&a, &[a.clone(), b.clone()]);
        let mut commands = parked(&mut router, model.clone());
        router.globals = model.clone();
        router.scope.select(Some(b.clone()));
        router.scope.advance().unwrap(); // UI intent queued, router has not selected yet.
        router.update(0, Update::Model(Box::new(model))).await;
        assert_eq!(model_update(visible.recv().await).1, None);
        assert!(router.scope.is_selected(&b));
        assert!(
            router
                .command(Command::Scoped {
                    generation: 0,
                    command: Box::new(Command::Request(Operation::TerminalInput {
                        stream: "same-stream".into(),
                        data: protocol::encode(b"must not send"),
                    })),
                })
                .await
                .is_err()
        );
        assert!(commands.try_recv().is_err());
        assert!(visible.try_recv().is_err());
    }

    #[tokio::test]
    async fn disconnect_activates_the_new_epoch_and_only_invalidates_its_owner() {
        let (mut router, mut visible, _events, _snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let bindings = [a.clone(), b.clone()];
        let mut active = parked(&mut router, ready(&a, &bindings));
        let mut background = parked(&mut router, ready(&b, &bindings));
        router.globals.bindings = bindings.to_vec();
        let epoch = router.scope.advance().unwrap();
        router
            .command(Command::Scoped {
                generation: epoch,
                command: Box::new(Command::Disconnect),
            })
            .await
            .unwrap();
        let Some(Command::Activate {
            epoch: activation, ..
        }) = active.recv().await
        else {
            panic!("activation barrier")
        };
        assert!(
            matches!(active.recv().await, Some(Command::Scoped { generation: 1, command }) if matches!(*command, Command::Disconnect))
        );
        assert!(background.try_recv().is_err());
        assert_eq!(
            router.workers[1].client.scope.as_ref().unwrap().current(),
            0
        );
        let mut model = router.workers[0].model.clone();
        model.activation = activation;
        model.generation = 1;
        model.phase = Phase::Offline;
        router.update(0, Update::Model(Box::new(model))).await;
        assert_eq!(model_update(visible.recv().await).1, Some(epoch));
        assert!(router.workers[1].model.ready());
    }

    #[tokio::test]
    async fn forgetting_a_background_host_updates_the_visible_list_without_detaching_it() {
        let (mut router, mut visible, _events, snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let bindings = [a.clone(), b.clone()];
        let mut first = ready(&a, &bindings);
        first.aliases = labels(&[(&a, "A"), (&b, "B")]);
        first.host_names = labels(&[(&a, "host-a"), (&b, "host-b")]);
        let mut second = first.clone();
        second.active_binding = Some(b.clone());
        let mut active = parked(&mut router, first.clone());
        let mut background = parked(&mut router, second.clone());
        router.globals = first;
        router.scope.select(Some(a.clone()));
        router.command(Command::Forget(b.clone())).await.unwrap();
        assert!(active.try_recv().is_err());
        assert!(
            matches!(background.recv().await, Some(Command::Scoped { generation: 1, command }) if matches!(*command, Command::Forget(ref forgotten) if forgotten == &b))
        );
        assert_eq!(
            router.workers[0].client.scope.as_ref().unwrap().current(),
            0
        );
        second.generation = 1;
        second.phase = Phase::Offline;
        second.active_binding = None;
        second.bindings.retain(|binding| binding != &b);
        second.aliases.retain(&second.bindings);
        second.host_names.retain(&second.bindings);
        commit(&snapshots, 1, &second);
        router.update(1, Update::Model(Box::new(second))).await;
        assert_eq!(model_update(visible.recv().await).1, None);
        let (workspace, selected, model) = model_update(visible.recv().await);
        assert_eq!((workspace, selected), (0, Some(0)));
        assert!(model.ready());
        assert_eq!(model.active_binding, Some(a.clone()));
        assert_eq!(model.bindings, vec![a.clone()]);
        assert_eq!(model.aliases.get(&a), Some("A"));
        assert_eq!(model.aliases.get(&b), None);
        assert_eq!(model.host_names.get(&b), None);
        assert!(router.workers[1].target == Target::Empty);
        assert!(router.scope.is_selected(&a));
    }

    #[tokio::test]
    async fn reordered_owner_confirmations_cannot_roll_back_a_later_persisted_alias_or_name() {
        let (mut router, _visible, _events, snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let mut baseline = ready(&a, &[a.clone(), b.clone()]);
        baseline.aliases = labels(&[(&a, "a0"), (&b, "b0")]);
        baseline.host_names = labels(&[(&a, "host-a0"), (&b, "host-b0")]);
        let _first = parked(&mut router, baseline.clone());
        let _second = parked(&mut router, baseline.clone());
        router.globals = baseline.clone();
        // Disk commits B1, A1, B2 in order. A's confirmed full table contains
        // the older B1, even though its actor publication can arrive last.
        let mut b1 = baseline.clone();
        b1.aliases = labels(&[(&a, "a0"), (&b, "b1")]);
        b1.host_names = labels(&[(&a, "host-a0"), (&b, "host-b1")]);
        commit(&snapshots, 1, &b1);
        let mut a1 = b1.clone();
        a1.aliases = labels(&[(&a, "a1"), (&b, "b1")]);
        a1.host_names = labels(&[(&a, "host-a1"), (&b, "host-b1")]);
        commit(&snapshots, 2, &a1);
        let mut b2 = a1.clone();
        b2.aliases = labels(&[(&a, "a1"), (&b, "b2")]);
        b2.host_names = labels(&[(&a, "host-a1"), (&b, "host-b2")]);
        commit(&snapshots, 3, &b2);
        for (id, model) in [(1, b1), (1, b2.clone()), (0, a1)] {
            router.update(id, Update::Model(Box::new(model))).await;
            assert_eq!(router.globals.aliases, b2.aliases);
            assert_eq!(router.globals.host_names, b2.host_names);
            assert_eq!(router.shared_revision, 3);
        }
        // Two different workers may even rename the same identity. Neither
        // alias_saving nor UI generation makes a delayed older write current.
        let mut delayed = baseline;
        delayed.alias_saving = Some(b);
        router.update(0, Update::Model(Box::new(delayed))).await;
        assert_eq!(router.globals.aliases, b2.aliases);
    }

    #[tokio::test]
    async fn a_forgotten_binding_cannot_be_resurrected_by_a_delayed_pairing_table_or_activation_echo()
     {
        let (mut router, _visible, _events, snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let c = binding(9);
        let mut baseline = ready(&a, std::slice::from_ref(&a));
        baseline.aliases = labels(&[(&a, "old-a")]);
        let mut commands = parked(&mut router, baseline.clone());
        let _second = parked(&mut router, baseline.clone());
        router.globals = baseline.clone();
        router.activate(0).unwrap();
        let Some(Command::Activate {
            epoch: activation, ..
        }) = commands.recv().await
        else {
            panic!("activation barrier");
        };
        let mut paired_b = baseline.clone();
        paired_b.bindings.push(b.clone());
        commit(&snapshots, 1, &paired_b);
        let mut delayed_pair_c = paired_b.clone();
        delayed_pair_c.bindings.push(c.clone());
        commit(&snapshots, 2, &delayed_pair_c);
        let mut forgotten_b = delayed_pair_c.clone();
        forgotten_b.bindings.retain(|binding| binding != &b);
        forgotten_b.aliases = labels(&[(&a, "new-a")]);
        commit(&snapshots, 3, &forgotten_b);
        router
            .update(1, Update::Model(Box::new(forgotten_b.clone())))
            .await;
        router
            .update(1, Update::Model(Box::new(delayed_pair_c)))
            .await;
        baseline.activation = activation;
        router.update(0, Update::Model(Box::new(baseline))).await;
        assert_eq!(router.globals.bindings, vec![a.clone(), c]);
        assert_eq!(router.globals.aliases.get(&a), Some("new-a"));
        assert!(!router.globals.bindings.contains(&b));
        assert_eq!(router.shared_revision, 3);
        // Even a malformed/reordered notification must not lower the revision.
        commit(&snapshots, 1, &paired_b);
        router.receive_saved().await;
        assert_eq!(router.globals.bindings, forgotten_b.bindings);
        assert_eq!(router.shared_revision, 3);
    }

    #[tokio::test]
    async fn watch_coalescing_refreshes_the_visible_list_without_any_actor_publication() {
        let (mut router, mut visible, _events, snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let baseline = ready(&a, std::slice::from_ref(&a));
        let mut commands = parked(&mut router, baseline.clone());
        router.globals = baseline.clone();
        let mut paired = baseline.clone();
        paired.bindings.push(b.clone());
        commit(&snapshots, 1, &paired);
        let mut latest = paired;
        latest.aliases = labels(&[(&a, "renamed-a"), (&b, "renamed-b")]);
        latest.host_names = labels(&[(&a, "host-a"), (&b, "host-b")]);
        commit(&snapshots, 2, &latest);
        storage::snapshots::changed(&mut router.saved).await;
        router.receive_saved().await;
        let (workspace, selected, model) = model_update(visible.recv().await);
        assert_eq!((workspace, selected), (0, Some(0)));
        assert_eq!(model.bindings, latest.bindings);
        assert_eq!(model.aliases, latest.aliases);
        assert_eq!(model.host_names, latest.host_names);
        assert!(model.ready());
        assert_eq!(router.shared_revision, 2);
        assert!(
            commands.try_recv().is_err(),
            "a background commit must not switch or detach a worker"
        );
        assert!(
            visible.try_recv().is_err(),
            "intermediate snapshots were coalesced"
        );
    }

    #[tokio::test]
    async fn command_admission_reads_the_latest_confirmed_pairings_before_routing() {
        let (mut router, _visible, _events, snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let baseline = ready(&a, &[a.clone(), b.clone()]);
        let mut active = parked(&mut router, baseline.clone());
        router.globals = baseline;
        let forgotten = ready(&a, std::slice::from_ref(&a));
        commit(&snapshots, 1, &forgotten);
        // The watch branch has not run. Reconnect must still reject the
        // forgotten identity before it reaches any private actor scope.
        assert!(router.command(Command::Reconnect(b)).await.is_err());
        assert_eq!(router.globals.bindings, vec![a]);
        assert_eq!(router.shared_revision, 1);
        assert!(active.try_recv().is_err());
    }

    #[tokio::test]
    async fn pairing_replacement_comes_only_from_the_confirmed_snapshot() {
        let (mut router, _visible, _events, snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let mut replacement = binding(9);
        replacement.route = a.route.clone();
        router.globals.bindings = vec![a.clone(), b.clone()];
        router.globals.aliases = labels(&[(&a, "old trusted host"), (&b, "keep")]);
        let _commands = parked(&mut router, Model::default());
        let unconfirmed = Model {
            bindings: vec![replacement.clone()],
            ..Model::default()
        };
        router
            .update(0, Update::Model(Box::new(unconfirmed.clone())))
            .await;
        assert_eq!(router.globals.bindings, vec![a.clone(), b.clone()]);
        let confirmed = Model {
            bindings: vec![b.clone(), replacement.clone()],
            aliases: labels(&[(&b, "keep")]),
            ..Model::default()
        };
        commit(&snapshots, 1, &confirmed);
        router.update(0, Update::Model(Box::new(unconfirmed))).await;
        assert_eq!(router.globals.bindings, confirmed.bindings);
        assert_eq!(router.globals.aliases.get(&a), None);
        assert_eq!(router.globals.aliases.get(&replacement), None);
        assert_eq!(router.globals.aliases.get(&b), Some("keep"));
    }

    #[tokio::test]
    async fn rapid_host_selection_reserves_workers_before_actor_updates_arrive() {
        let (mut router, _visible, _events, _snapshots) = fixture();
        let a = binding(1);
        let b = binding(5);
        let mut first = parked(&mut router, Model::default());
        let mut second = parked(&mut router, Model::default());
        router.globals.bindings = vec![a.clone(), b.clone()];
        for binding in [a.clone(), b.clone()] {
            let generation = router.scope.advance().unwrap();
            router
                .command(Command::Scoped {
                    generation,
                    command: Box::new(Command::Reconnect(binding)),
                })
                .await
                .unwrap();
        }
        assert_eq!(router.active, 1);
        assert!(router.workers[0].target == Target::Remote(a.clone()));
        assert!(router.workers[1].target == Target::Remote(b.clone()));
        for (commands, binding) in [(&mut first, a), (&mut second, b)] {
            assert!(matches!(
                commands.recv().await,
                Some(Command::Activate { .. })
            ));
            assert!(
                matches!(commands.recv().await, Some(Command::Scoped { generation: 1, command }) if matches!(*command, Command::Reconnect(ref target) if target == &binding))
            );
            assert!(commands.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn output_is_namespaced_in_the_background_and_old_connection_generations_are_discarded() {
        let (mut router, mut visible, _events, _snapshots) = fixture();
        let _first = parked(&mut router, Model::default());
        let _second = parked(&mut router, Model::default());
        for id in [0, 1] {
            router
                .update(
                    id,
                    Update::Output {
                        generation: 0,
                        stream: "same-stream".into(),
                        text: format!("worker-{id}"),
                        reset: false,
                    },
                )
                .await;
            assert!(
                matches!(visible.recv().await, Some(Update::WorkspaceOutput { workspace, stream, text, .. }) if workspace == id as u64 && stream == "same-stream" && text == format!("worker-{id}"))
            );
        }
        let model = Model {
            generation: 1,
            ..Model::default()
        };
        router.update(1, Update::Model(Box::new(model))).await;
        let _ = visible.recv().await;
        router
            .update(
                1,
                Update::Output {
                    generation: 0,
                    stream: "same-stream".into(),
                    text: "stale".into(),
                    reset: true,
                },
            )
            .await;
        router.update(1, Update::Model(Box::default())).await;
        assert!(visible.try_recv().is_err());
        assert_eq!(router.workers[1].model.generation, 1);
        router
            .update(
                1,
                Update::Output {
                    generation: 1,
                    stream: "same-stream".into(),
                    text: "new".into(),
                    reset: true,
                },
            )
            .await;
        assert!(
            matches!(visible.recv().await, Some(Update::WorkspaceOutput { workspace: 1, generation: 1, reset: true, text, .. }) if text == "new")
        );
    }

    #[tokio::test]
    async fn superseding_a_warm_selection_keeps_the_writer_until_the_os_write_and_snapshot_ack() {
        let scope = Arc::new(ConnectionScope::default());
        let writer = Arc::new(tokio::sync::Mutex::new(()));
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let (snapshots, mut saved) = tokio::sync::watch::channel(None);
        let disk_written = Arc::new(AtomicBool::new(false));
        let dispatched = Arc::new(AtomicBool::new(false));
        let task_scope = scope.clone();
        let task_writer = writer.clone();
        let task_written = disk_written.clone();
        let task_dispatched = dispatched.clone();
        let task = tokio::spawn(async move {
            admitted(&task_scope, 0, async {
                let _guard = task_writer.lock().await;
                // Model the native secure-store spawn_blocking job. Cancelling
                // its JoinHandle future cannot stop an already running OS job.
                tokio::task::spawn_blocking(move || {
                    let _ = started.send(());
                    blocked.recv().context("fixture write released")?;
                    task_written.store(true, Ordering::Release);
                    Ok::<_, anyhow::Error>(())
                })
                .await??;
                commit(&snapshots, 1, &Model::default());
                // Warm selection already rechecks here before selecting a UI.
                ensure!(task_scope.current() == 0, "superseded warm selection");
                task_dispatched.store(true, Ordering::Release);
                Ok(())
            })
            .await
        });
        waiting.await.unwrap();
        scope.advance().unwrap();
        tokio::task::yield_now().await;
        let writer_still_held = writer.try_lock().is_err();
        let route_still_running = !task.is_finished();
        // Always release the task-owned blocking job before assertions.
        release.send(()).unwrap();
        assert!(
            task.await.unwrap().is_err(),
            "superseded intent must not report success"
        );
        assert!(
            writer_still_held && route_still_running,
            "scope changes must not drop the writer guard while the OS write continues"
        );
        assert!(disk_written.load(Ordering::Acquire));
        assert_eq!(saved.borrow_and_update().as_ref().unwrap().revision, 1);
        assert!(
            !dispatched.load(Ordering::Acquire),
            "a completed stale write must not select a newer UI"
        );
        assert!(writer.try_lock().is_ok());
    }

    #[tokio::test]
    async fn stale_router_admission_never_polls_persistence_or_dispatch() {
        let scope = ConnectionScope::default();
        scope.advance().unwrap();
        let polled = AtomicBool::new(false);
        assert!(
            admitted(&scope, 0, async {
                polled.store(true, Ordering::Release);
                Ok(())
            })
            .await
            .is_err()
        );
        assert!(!polled.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn a_completed_route_does_not_acknowledge_a_superseded_generation() {
        let scope = ConnectionScope::default();
        let completed = AtomicBool::new(false);
        assert!(
            admitted(&scope, 0, async {
                scope.advance()?;
                completed.store(true, Ordering::Release);
                Ok(())
            })
            .await
            .is_err()
        );
        assert!(completed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn failed_activation_does_not_select_another_workspace_or_advance_markers() {
        let (mut router, _visible, _events, _snapshots) = fixture();
        let _first = parked(&mut router, Model::default());
        let _second = parked(&mut router, Model::default());
        for _ in 0..QUEUE {
            router.workers[1]
                .client
                .sender
                .try_send(Command::ConfigureTerminal {
                    remote_auto_control: false,
                })
                .unwrap();
        }
        router.scope.advance().unwrap();
        assert!(router.select(1).await.is_err());
        assert_eq!((router.active, router.epoch, router.activation), (0, 0, 0));
        assert_eq!(router.workers[1].activation, 0);
    }

    #[tokio::test]
    async fn suspend_is_global_even_after_a_newer_ui_selection() {
        let (mut router, _visible, _events, _snapshots) = fixture();
        let mut commands = parked(&mut router, Model::default());
        router.scope.advance().unwrap();
        router
            .command(Command::Scoped {
                generation: 0,
                command: Box::new(Command::Suspend),
            })
            .await
            .unwrap();
        assert!(router.stopping);
        assert!(
            commands.try_recv().is_err(),
            "shutdown closes the actor instead of leaving a suspended loop alive"
        );
    }

    #[tokio::test]
    async fn shutdown_joins_actors_cancels_preparations_and_closes_real_transports() {
        let (mut router, _visible, _events, _snapshots) = fixture();
        let mut observed = Vec::new();
        let mut cancelled = Vec::new();
        let mut handles = Vec::new();
        for _ in 0..2 {
            let (updates, receiver) = mpsc::channel(1);
            let mut actor = Actor::new(updates);
            let (session, requests) = Session::observed();
            actor.session = Some(session.into());
            let cancel = Arc::new(AtomicBool::new(false));
            actor.preparation = Some(Preparation {
                generation: 0,
                cancel: cancel.clone(),
                grant: "test".into(),
                path: "test".into(),
            });
            actor.outstanding.insert(
                1,
                Outstanding {
                    kind: Pending::Ordinary,
                    expires: Instant::now() + Duration::from_secs(60),
                },
            );
            let id = router.spawn(actor, receiver);
            handles.push((
                router.workers[id].actor.abort_handle(),
                router.workers[id].forwarding.abort_handle(),
            ));
            observed.push(requests);
            cancelled.push(cancel);
        }
        tokio::time::timeout(Duration::from_secs(3), router.shutdown())
            .await
            .unwrap();
        assert!(router.workers.is_empty());
        assert!(
            cancelled.iter().all(|flag| flag.load(Ordering::Acquire)),
            "Actor::disconnected must cancel preparations before returning"
        );
        assert!(
            handles
                .iter()
                .all(|(actor, forwarding)| actor.is_finished() && forwarding.is_finished())
        );
        for mut requests in observed {
            assert!(
                requests.recv().await.is_none(),
                "shutdown must not replay, control, or close a host PTY"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_inflight_scope_work_and_joins_with_forwarding_backpressure() {
        let (mut router, _visible, _events, _snapshots) = fixture();
        let commands = parked(&mut router, Model::default());
        let worker = &mut router.workers[0];
        worker.actor.abort();
        let scope = worker.client.scope.as_ref().unwrap().clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let observed = cancelled.clone();
        worker.actor = tokio::spawn(async move {
            assert!(
                scope
                    .during(0, std::future::pending::<Result<()>>())
                    .await
                    .is_err()
            );
            observed.store(true, Ordering::Release);
        });
        // Hold the worker's command receiver open as well. Queue closure is
        // insufficient to stop an operation already waiting inside scope.during.
        for _ in 0..128 {
            router
                .incoming
                .try_send((0, Update::Model(Box::default())))
                .unwrap();
        }
        tokio::task::yield_now().await;
        tokio::time::timeout(Duration::from_secs(3), router.shutdown())
            .await
            .unwrap();
        assert!(cancelled.load(Ordering::Acquire));
        drop(commands);
    }
    #[tokio::test]
    async fn selecting_another_workspace_keeps_background_transport_output_and_control() {
        let (updates, mut visible) = mpsc::channel(64);
        let (incoming, mut events) = mpsc::channel(64);
        let scope = Arc::new(ConnectionScope::default());
        let (_snapshots, saved) = tokio::sync::watch::channel(None);
        let mut router = Workspaces {
            workers: Vec::new(),
            active: 0,
            epoch: 0,
            activation: 0,
            stopping: false,
            globals: Model::default(),
            saved,
            shared_revision: 0,
            auto_control: false,
            updates,
            incoming,
            scope,
            #[cfg(not(target_os = "android"))]
            host: None,
        };
        let mut wire = Vec::new();
        for index in 0..2 {
            let (updates, receiver) = mpsc::channel(64);
            let mut actor = Actor::new(updates);
            actor.model.phase = Phase::Ready;
            actor.model.local = index == 0;
            actor.model.permission = Some(Permission::Operate);
            let (session, requests) = Session::observed();
            actor.session = Some(session.into());
            router.spawn(actor, receiver);
            wire.push(requests);
        }
        for (id, worker) in router.workers.iter().enumerate() {
            assert!(worker.model.ready(), "worker {id} starts ready");
        }
        for id in [1, 0, 1, 0] {
            router.scope.advance().unwrap();
            router.select(id).await.unwrap();
            let (source, update) = events.recv().await.unwrap();
            assert_eq!(source, id);
            router.update(source, update).await;
            let Some(Update::WorkspaceModel {
                workspace,
                selected,
                ..
            }) = visible.recv().await
            else {
                panic!("workspace model")
            };
            assert_eq!(workspace, id as u64);
            assert_eq!(selected, Some(router.scope.current()));
            for wire in &mut wire {
                while let Ok(request) = wire.try_recv() {
                    assert!(
                        matches!(request.operation, Operation::Ping { .. }),
                        "selection must not identify, detach, reconnect, or replay input: {:?}",
                        request.operation,
                    );
                }
            }
        }
        router
            .update(
                1,
                Update::Output {
                    generation: 0,
                    stream: "remote".into(),
                    text: "still running".into(),
                    reset: false,
                },
            )
            .await;
        assert!(
            matches!(visible.recv().await,Some(Update::WorkspaceOutput {workspace:1,text,..}) if text == "still running")
        );
        assert!(router.workers.iter().all(|worker| worker.model.ready()));
    }
}
