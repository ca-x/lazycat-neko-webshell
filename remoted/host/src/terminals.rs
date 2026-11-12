//! Local PTYs shared by local and remote subscribers. Detachment never kills a shell.
use anyhow::{Context, Result, bail, ensure};
use neko_remoted_protocol::{self as protocol, messages::Event};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex, MutexGuard, Weak, mpsc as blocking},
    thread,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

mod io;
mod program_status;
mod replies;
use io::{read_output, write_commands};
use program_status::ProgramStatus;

#[cfg(test)]
use io::accept_output;
#[cfg(test)]
use std::io::Read;

#[cfg(test)]
mod program_status_tests;

#[cfg(all(test, unix))]
mod resize_tests;

const MAX_SESSIONS: usize = 32;
const MAX_ATTACHMENTS: usize = 64;
const REPLAY_BYTES: usize = 256 * 1024;
const CHUNK: usize = 16 * 1024;
const EVENT_QUEUE: usize = 32;
const IO_QUEUE: usize = 32;
const IO_DEADLINE: Duration = Duration::from_secs(5);
const DELIVERY_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
pub struct TerminalRegistry {
    inner: Arc<Registry>,
}
#[derive(Default)]
struct Registry {
    state: Mutex<RegistryState>,
}
#[derive(Default)]
struct RegistryState {
    terminals: HashMap<String, Arc<Terminal>>,
    creating: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalInfo {
    pub id: String,
    pub name: String,
    pub exit: Option<i32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub stream: String,
    pub name: String,
    pub read_only: bool,
}
// The waiter exclusively owns the unreaped child. Never use portable-pty's Unix
// clone_killer here: it only sends SIGHUP and cannot terminate a HUP handler.
// Forward a bounded request to that owner, which uses the original child handle.
#[derive(Clone, Debug)]
struct TerminationRequest(blocking::SyncSender<()>);
impl ChildKiller for TerminationRequest {
    fn kill(&mut self) -> std::io::Result<()> {
        // Repeated requests coalesce; the authoritative exited watch below, not
        // enqueue success, determines whether close succeeded.
        let _ = self.0.try_send(());
        Ok(())
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}
struct Terminal {
    name: String,
    shared: Arc<Shared>,
    io: blocking::SyncSender<IoCommand>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    exited: watch::Receiver<Option<std::result::Result<i32, String>>>,
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // Reader/writer/waiter hold Shared, never Terminal: the last registry owner
        // therefore releases a live child even if an output reader is blocked.
        if let Ok(state) = self.shared.state.lock()
            && state.exit.is_some()
        {
            return;
        }
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
    }
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    drained: Condvar,
}
#[derive(Default)]
struct State {
    program_status: ProgramStatus,
    dimensions: (u16, u16),
    replay: VecDeque<u8>,
    truncated: bool,
    attachments: HashMap<String, Subscriber>,
    controller: Option<String>,
    closing: bool,
    exit: Option<i32>,
    read_done: bool,
    finished: bool,
    failure: Option<String>,
}
struct Subscriber {
    owner: String,
    read_only: bool,
    events: mpsc::Sender<Event>,
    stop: watch::Sender<Stop>,
    sequence: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Running,
    Detached,
    Gap,
}
struct DeliveryGuard {
    shared: Weak<Shared>,
    stream: String,
}
impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade()
            && let Ok(mut state) = shared.state.lock()
        {
            remove_subscriber(&mut state, &self.stream, Stop::Detached);
        }
    }
}
// Reap the child even if creating a reader/writer/waiter thread fails.
struct ChildGuard(Box<dyn portable_pty::Child + Send + Sync>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

struct IoCommand {
    stream: String,
    subscriber: String,
    action: IoAction,
    completed: oneshot::Sender<Result<()>>,
}
enum IoAction {
    Input(Vec<u8>),
    Resize(PtySize),
}
struct ReplyAuthority {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    replies: blocking::SyncSender<Vec<u8>>,
}
impl ReplyAuthority {
    fn enqueue(&self, reply: Vec<u8>) -> Result<()> {
        if !reply.is_empty() {
            self.replies
                .try_send(reply)
                .context("PTY support reply queue full or closed")?;
        }
        Ok(())
    }

    fn foreground(&self, status: &mut ProgramStatus) {
        if let Ok(master) = self.master.lock()
            && let Ok(dimensions) = master.get_size()
        {
            #[cfg(unix)]
            let group = master
                .process_group_leader()
                .and_then(|pid| u32::try_from(pid).ok());
            #[cfg(not(unix))]
            let group = None;
            status.foreground(group, dimensions.cols, dimensions.rows);
        }
    }
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| anyhow::anyhow!("terminal state lock poisoned"))
}
fn size(cols: u16, rows: u16) -> Result<PtySize> {
    replies::dimensions(cols, rows)?;
    Ok(PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    })
}
struct Reservation {
    registry: Arc<Registry>,
    active: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.active
            && let Ok(mut state) = self.registry.state.lock()
        {
            state.creating -= 1;
        }
    }
}

impl TerminalRegistry {
    pub async fn create(&self, name: String, cols: u16, rows: u16) -> Result<String> {
        self.create_with_command(name, cols, rows, CommandBuilder::new_default_prog())
            .await
    }

    async fn create_with_command(
        &self,
        name: String,
        cols: u16,
        rows: u16,
        command: CommandBuilder,
    ) -> Result<String> {
        ensure!(
            !name.trim().is_empty() && name.len() <= 160 && !name.chars().any(char::is_control),
            "invalid terminal name"
        );
        let dimensions = size(cols, rows)?;
        let reservation = {
            let mut state = lock(&self.inner.state)?;
            ensure!(
                state.terminals.len() + state.creating < MAX_SESSIONS,
                "terminal session limit reached"
            );
            state.creating += 1;
            Reservation {
                registry: self.inner.clone(),
                active: true,
            }
        };
        // The reservation moves into the worker, so cancellation cannot release
        // capacity while a blocking PTY creation is still in progress.
        let (mut reservation, terminal) = tokio::task::spawn_blocking(move || {
            let terminal = spawn_terminal(name, dimensions, command)?;
            Ok::<_, anyhow::Error>((reservation, terminal))
        })
        .await
        .context("PTY creation worker stopped")??;
        let id = Uuid::new_v4().to_string();
        let mut state = lock(&self.inner.state)?;
        state.terminals.insert(id.clone(), Arc::new(terminal));
        state.creating -= 1;
        reservation.active = false;
        Ok(id)
    }

    pub fn list(&self) -> Result<Vec<TerminalInfo>> {
        let registry = lock(&self.inner.state)?;
        let mut result = registry
            .terminals
            .iter()
            .map(|(id, terminal)| {
                Ok(TerminalInfo {
                    id: id.clone(),
                    name: terminal.name.clone(),
                    exit: lock(&terminal.shared.state)?.exit,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        result.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }

    pub async fn attach(
        &self,
        terminal: &str,
        subscriber: String,
        events: mpsc::Sender<Event>,
        read_only: bool,
    ) -> Result<Attachment> {
        ensure!(
            !subscriber.is_empty() && subscriber.len() <= 256,
            "invalid subscriber identity"
        );
        ensure!(!events.is_closed(), "subscriber event queue is closed");
        let terminal = self.terminal(terminal)?;
        let stream = Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel(EVENT_QUEUE);
        let (stop, stopped) = watch::channel(Stop::Running);
        {
            let mut state = lock(&terminal.shared.state)?;
            ensure!(!state.closing, "terminal is closing");
            ensure!(
                state.attachments.len() < MAX_ATTACHMENTS,
                "terminal attachment limit reached"
            );
            let mut entry = Subscriber {
                owner: subscriber,
                read_only,
                events: tx,
                stop,
                sequence: 0,
            };
            queue_output(&mut entry, &stream, &[], true, true, state.truncated)?;
            // Snapshot enqueue and subscription installation share the reader's
            // lock. Live bytes cannot overtake replay or fall into a subscribe gap.
            let snapshot: Vec<u8> = state.replay.iter().copied().collect();
            for chunk in snapshot.chunks(CHUNK) {
                queue_output(&mut entry, &stream, chunk, true, false, false)?;
            }
            entry
                .events
                .try_send(Event::TerminalProgramStatus {
                    stream: stream.clone(),
                    records: state.program_status.records().to_vec(),
                })
                .context("status replay queue full")?;
            if let Some(error) = &state.failure {
                entry
                    .events
                    .try_send(failure_event(error))
                    .context("replay queue full")?;
            }
            if state.finished
                && let Some(code) = state.exit
            {
                entry
                    .events
                    .try_send(Event::TerminalExit {
                        stream: stream.clone(),
                        code,
                    })
                    .context("replay queue full")?;
            }
            state.attachments.insert(stream.clone(), entry);
        }
        tokio::spawn(deliver(
            DeliveryGuard {
                shared: Arc::downgrade(&terminal.shared),
                stream: stream.clone(),
            },
            stream.clone(),
            rx,
            stopped,
            events,
        ));
        Ok(Attachment {
            stream,
            name: terminal.name.clone(),
            read_only,
        })
    }

    pub fn control(&self, stream: &str, subscriber: &str, take: bool) -> Result<bool> {
        let terminal = self.for_stream(stream)?;
        let mut state = lock(&terminal.shared.state)?;
        let owner = owned(&state, stream, subscriber)?;
        if take {
            ensure!(!owner.read_only, "terminal attachment is read-only");
            ensure!(
                !state.closing && state.exit.is_none() && state.failure.is_none(),
                "terminal is not running"
            );
            if state.controller.as_deref() != Some(stream) {
                let old = state.controller.take();
                if let Some(old) = old {
                    queue_event(
                        &mut state,
                        &old,
                        Event::TerminalControl {
                            stream: old.clone(),
                            controller: false,
                        },
                    );
                }
                state.controller = Some(stream.to_owned());
                queue_event(
                    &mut state,
                    stream,
                    Event::TerminalControl {
                        stream: stream.into(),
                        controller: true,
                    },
                );
                ensure!(
                    state.controller.as_deref() == Some(stream),
                    "subscriber output gap; reattach terminal"
                );
            }
        } else if state.controller.as_deref() == Some(stream) {
            state.controller = None;
            queue_event(
                &mut state,
                stream,
                Event::TerminalControl {
                    stream: stream.into(),
                    controller: false,
                },
            );
        }
        Ok(state.controller.as_deref() == Some(stream))
    }

    pub async fn input(&self, stream: &str, subscriber: &str, data: Vec<u8>) -> Result<()> {
        ensure!(
            !data.is_empty() && data.len() <= CHUNK,
            "terminal input must contain 1..16384 bytes"
        );
        self.io(stream, subscriber, IoAction::Input(data)).await
    }

    pub async fn resize(&self, stream: &str, subscriber: &str, cols: u16, rows: u16) -> Result<()> {
        self.io(stream, subscriber, IoAction::Resize(size(cols, rows)?))
            .await
    }

    pub fn detach(&self, stream: &str, subscriber: &str) -> Result<()> {
        let terminal = self.for_stream(stream)?;
        let mut state = lock(&terminal.shared.state)?;
        owned(&state, stream, subscriber)?;
        remove_subscriber(&mut state, stream, Stop::Detached);
        Ok(())
    }

    pub fn detach_all(&self, subscriber: &str) {
        if let Ok(registry) = self.inner.state.lock() {
            for terminal in registry.terminals.values() {
                if let Ok(mut state) = terminal.shared.state.lock() {
                    let streams: Vec<_> = state
                        .attachments
                        .iter()
                        .filter(|(_, item)| item.owner == subscriber)
                        .map(|(id, _)| id.clone())
                        .collect();
                    for stream in streams {
                        remove_subscriber(&mut state, &stream, Stop::Detached);
                    }
                }
            }
        }
    }

    pub async fn close(&self, id: &str) -> Result<()> {
        let terminal = self.terminal(id)?;
        let running = {
            let mut state = lock(&terminal.shared.state)?;
            state.closing = true;
            state.exit.is_none()
        };
        if running {
            let child = terminal.clone();
            tokio::task::spawn_blocking(move || {
                lock(&child.killer)?
                    .kill()
                    .context("cannot terminate PTY child")
            })
            .await
            .context("PTY termination worker stopped")??;
        }
        let mut exited = terminal.exited.clone();
        tokio::time::timeout(IO_DEADLINE, async {
            loop {
                if let Some(result) = exited.borrow().clone() {
                    return result.map_err(anyhow::Error::msg);
                }
                exited
                    .changed()
                    .await
                    .context("PTY wait worker stopped without exit status")?;
            }
        })
        .await
        .context("PTY child termination not confirmed; session retained")??;
        lock(&self.inner.state)?.terminals.remove(id);
        Ok(())
    }

    fn terminal(&self, id: &str) -> Result<Arc<Terminal>> {
        lock(&self.inner.state)?
            .terminals
            .get(id)
            .cloned()
            .context("unknown terminal")
    }

    fn for_stream(&self, stream: &str) -> Result<Arc<Terminal>> {
        let registry = lock(&self.inner.state)?;
        for terminal in registry.terminals.values() {
            if lock(&terminal.shared.state)?
                .attachments
                .contains_key(stream)
            {
                return Ok(terminal.clone());
            }
        }
        bail!("unknown or detached terminal stream; reattach after an output gap")
    }

    async fn io(&self, stream: &str, subscriber: &str, action: IoAction) -> Result<()> {
        let terminal = self.for_stream(stream)?;
        writable(&*lock(&terminal.shared.state)?, stream, subscriber)?;
        let (completed, result) = oneshot::channel();
        terminal
            .io
            .try_send(IoCommand {
                stream: stream.into(),
                subscriber: subscriber.into(),
                action,
                completed,
            })
            .map_err(|_| {
                anyhow::anyhow!("terminal input queue full or closed; request was not queued")
            })?;
        tokio::time::timeout(IO_DEADLINE, result)
            .await
            .context("PTY operation timed out; outcome unknown, do not replay")?
            .context("PTY I/O worker stopped; outcome unknown, do not replay")?
    }
}

fn owned<'a>(state: &'a State, stream: &str, subscriber: &str) -> Result<&'a Subscriber> {
    let owner = state
        .attachments
        .get(stream)
        .context("terminal attachment is detached")?;
    ensure!(
        owner.owner == subscriber,
        "terminal attachment belongs to another subscriber"
    );
    Ok(owner)
}
fn writable(state: &State, stream: &str, subscriber: &str) -> Result<()> {
    ensure!(
        !owned(state, stream, subscriber)?.read_only,
        "terminal attachment is read-only"
    );
    ensure!(
        state.controller.as_deref() == Some(stream),
        "terminal control is not held by this attachment"
    );
    ensure!(
        !state.closing && state.exit.is_none() && state.failure.is_none(),
        "terminal is not running"
    );
    Ok(())
}
fn remove_subscriber(state: &mut State, stream: &str, reason: Stop) {
    if let Some(entry) = state.attachments.remove(stream) {
        entry.stop.send_replace(reason);
    }
    if state.controller.as_deref() == Some(stream) {
        state.controller = None;
    }
}
fn queue_output(
    entry: &mut Subscriber,
    stream: &str,
    bytes: &[u8],
    replay: bool,
    reset: bool,
    gap: bool,
) -> Result<()> {
    entry.sequence = entry
        .sequence
        .checked_add(1)
        .context("terminal sequence exhausted")?;
    entry
        .events
        .try_send(Event::TerminalOutput {
            stream: stream.into(),
            sequence: entry.sequence,
            data: protocol::encode(bytes),
            replay,
            reset,
            gap,
        })
        .context("terminal subscriber queue full or closed")
}
fn queue_event(state: &mut State, stream: &str, event: Event) {
    if state
        .attachments
        .get(stream)
        .is_some_and(|entry| entry.events.try_send(event).is_err())
    {
        remove_subscriber(state, stream, Stop::Gap);
    }
}
fn publish_program_status(state: &mut State) {
    if !state.program_status.changed() {
        return;
    }
    let streams: Vec<_> = state.attachments.keys().cloned().collect();
    for stream in streams {
        let event = Event::TerminalProgramStatus {
            stream: stream.clone(),
            records: state.program_status.records().to_vec(),
        };
        queue_event(state, &stream, event);
    }
}
fn failure_event(error: &str) -> Event {
    Event::Error {
        code: "TERMINAL_ERROR".into(),
        message: error.into(),
        outcome_unknown: false,
    }
}

async fn stop_requested(stopped: &mut watch::Receiver<Stop>) {
    loop {
        if *stopped.borrow_and_update() != Stop::Running {
            return;
        }
        if stopped.changed().await.is_err() {
            // Normal registry teardown drops the stop sender after queueing the
            // final Exit. Keep draining incoming; closure is not an explicit detach.
            std::future::pending::<()>().await;
        }
    }
}

async fn deliver(
    guard: DeliveryGuard,
    stream: String,
    mut incoming: mpsc::Receiver<Event>,
    mut stopped: watch::Receiver<Stop>,
    outgoing: mpsc::Sender<Event>,
) {
    let mut sequence = 0;
    loop {
        let reason = *stopped.borrow();
        if reason != Stop::Running {
            if reason == Stop::Gap {
                let _ = tokio::time::timeout(
                    DELIVERY_DEADLINE,
                    outgoing.send(Event::TerminalOutput {
                        stream: stream.clone(),
                        sequence: sequence + 1,
                        data: String::new(),
                        replay: false,
                        reset: false,
                        gap: true,
                    }),
                )
                .await;
            }
            return;
        }
        let event = tokio::select! {
            biased;
            () = stop_requested(&mut stopped) => continue,
            () = outgoing.closed() => return,
            event = incoming.recv() => match event { Some(event) => event, None => return },
        };
        let next = match &event {
            Event::TerminalOutput { sequence, .. } => Some(*sequence),
            _ => None,
        };
        let sent = tokio::select! {
            biased;
            () = stop_requested(&mut stopped) => continue,
            sent = tokio::time::timeout(DELIVERY_DEADLINE, outgoing.send(event)) => sent,
        };
        if !matches!(sent, Ok(Ok(()))) {
            if let Some(shared) = guard.shared.upgrade()
                && let Ok(mut state) = shared.state.lock()
            {
                remove_subscriber(&mut state, &stream, Stop::Gap);
            }
            // A blocked external queue gets a bounded final gap attempt too.
            let _ = tokio::time::timeout(
                DELIVERY_DEADLINE,
                outgoing.send(Event::TerminalOutput {
                    stream,
                    sequence: sequence + 1,
                    data: String::new(),
                    replay: false,
                    reset: false,
                    gap: true,
                }),
            )
            .await;
            return;
        }
        if let Some(next) = next {
            sequence = next;
        }
    }
}

fn spawn_terminal(
    name: String,
    dimensions: PtySize,
    mut command: CommandBuilder,
) -> Result<Terminal> {
    let pair = native_pty_system()
        .openpty(dimensions)
        .context("cannot allocate PTY")?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .context("cannot open PTY output")?;
    let writer = pair.master.take_writer().context("cannot open PTY input")?;
    let (replies, reply_input) = blocking::sync_channel(IO_QUEUE);
    let authority = ReplyAuthority {
        master: Arc::new(Mutex::new(pair.master)),
        replies,
    };
    // portable-pty chooses the user's default login shell on Unix, ComSpec on
    // Windows. Environment/cwd changes apply only to this command's child.
    command.env("TERM", "xterm-256color");
    let mut child = ChildGuard(
        pair.slave
            .spawn_command(command)
            .context("cannot start default shell")?,
    );
    drop(pair.slave);
    let (terminate, terminating) = blocking::sync_channel(1);
    let killer: Box<dyn ChildKiller + Send + Sync> = Box::new(TerminationRequest(terminate));
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            dimensions: (dimensions.cols, dimensions.rows),
            ..State::default()
        }),
        ..Shared::default()
    });
    let (io, commands) = blocking::sync_channel(IO_QUEUE);
    let (exit_tx, exited) = watch::channel(None);
    let terminal = Terminal {
        name,
        shared: shared.clone(),
        io,
        killer: Mutex::new(killer),
        exited,
    };
    let (output, chunks) = blocking::sync_channel(IO_QUEUE);
    thread::Builder::new()
        .name("native-pty-output".into())
        .spawn(move || {
            read_output(&mut reader, output);
        })
        .context("cannot start PTY output worker")?;
    let input = shared.clone();
    thread::Builder::new()
        .name("native-pty-io".into())
        .spawn(move || {
            write_commands(writer, commands, chunks, reply_input, &input, &authority);
        })
        .context("cannot start PTY input worker")?;
    thread::Builder::new()
        .name("native-pty-wait".into())
        .spawn(move || {
            let result = wait_owned_child(&mut child, terminating)
                .map(|status| i32::from_ne_bytes(status.exit_code().to_ne_bytes()))
                .map_err(|e| format!("PTY child termination/wait failed: {e}"));
            finish(&shared, &result);
            exit_tx.send_replace(Some(result));
        })
        .context("cannot start PTY wait worker")?;
    Ok(terminal)
}

fn wait_owned_child(
    child: &mut ChildGuard,
    terminating: blocking::Receiver<()>,
) -> std::io::Result<portable_pty::ExitStatus> {
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(status);
        }
        match terminating.recv_timeout(Duration::from_millis(50)) {
            Ok(()) | Err(blocking::RecvTimeoutError::Disconnected) => {
                // The original portable-pty Unix child sends HUP, allows 200ms,
                // then uses std::process::Child::kill (SIGKILL). Windows uses its
                // owned process handle. This thread alone reaps: no PID can be
                // reused between our live-child check and the termination call.
                child.0.kill()?;
                return child.0.wait();
            }
            Err(blocking::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn finish(shared: &Shared, result: &std::result::Result<i32, String>) {
    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    match result {
        Ok(code) => state.exit = Some(*code),
        Err(error) => state.failure = Some(error.clone()),
    }
    // Drain final PTY bytes before announcing exit. Descendants retaining the
    // slave cannot delay termination forever: mark a gap after a bounded drain.
    if !state.read_done {
        let Ok((next, _)) = shared
            .drained
            .wait_timeout_while(state, DELIVERY_DEADLINE, |state| !state.read_done)
        else {
            return;
        };
        state = next;
    }
    state.finished = true;
    state.controller = None;
    let streams: Vec<_> = state.attachments.keys().cloned().collect();
    for stream in streams {
        if !state.read_done
            && let Some(entry) = state.attachments.get_mut(&stream)
            && queue_output(entry, &stream, &[], false, false, true).is_err()
        {
            remove_subscriber(&mut state, &stream, Stop::Gap);
        }
        let event = match result {
            Ok(code) => Event::TerminalExit {
                stream: stream.clone(),
                code: *code,
            },
            Err(error) => failure_event(error),
        };
        queue_event(&mut state, &stream, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Shell {
        registry: TerminalRegistry,
        id: String,
    }
    impl Drop for Shell {
        fn drop(&mut self) {
            // Cleanup on assertion failure too, without requiring an async Drop.
            if let Ok(terminal) = self.registry.terminal(&self.id)
                && let Ok(mut killer) = terminal.killer.lock()
            {
                let _ = killer.kill();
            }
        }
    }
    impl Shell {
        async fn new() -> Self {
            let registry = TerminalRegistry::default();
            // Isolate real-shell assertions from personal login profiles and
            // terminal capability probes; never mutate the process environment.
            #[cfg(unix)]
            let command = CommandBuilder::new("/bin/sh");
            #[cfg(windows)]
            let command = CommandBuilder::new_default_prog();
            let id = registry
                .create_with_command("real shell".into(), 80, 24, command)
                .await
                .unwrap();
            Self { registry, id }
        }
        async fn attach(
            &self,
            subscriber: &str,
            read_only: bool,
        ) -> (Attachment, mpsc::Receiver<Event>) {
            let (events, receiver) = mpsc::channel(128);
            let attachment = self
                .registry
                .attach(&self.id, subscriber.into(), events, read_only)
                .await
                .unwrap();
            (attachment, receiver)
        }
    }
    async fn output_line(events: &mut mpsc::Receiver<Event>, expected: &str) {
        let mut bytes = Vec::new();
        let received = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                match events
                    .recv()
                    .await
                    .expect("event stream closed before output")
                {
                    Event::TerminalOutput { data, .. } => {
                        bytes.extend(protocol::decode(&data, CHUNK).unwrap());
                        if String::from_utf8_lossy(&bytes)
                            .lines()
                            .any(|line| line.trim_end_matches('\r') == expected)
                        {
                            return;
                        }
                    }
                    Event::Error { message, .. } => panic!("PTY error: {message}"),
                    _ => {}
                }
            }
        })
        .await;
        assert!(
            received.is_ok(),
            "independent real PTY output line missing; tail={:?}",
            String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4096)..])
        );
    }
    fn print_command(marker: &str) -> Vec<u8> {
        #[cfg(unix)]
        {
            format!("printf '\\n%s\\n' '{marker}'\r").into_bytes()
        }
        #[cfg(windows)]
        {
            format!("echo {marker}\r").into_bytes()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_login_shell_creates_and_closes_without_global_changes() {
        let before_shell = std::env::var_os("SHELL");
        let before_cwd = std::env::current_dir().unwrap();
        let registry = TerminalRegistry::default();
        let id = registry
            .create("default login shell".into(), 80, 24)
            .await
            .unwrap();
        let shell = Shell { registry, id };
        assert_eq!(shell.registry.list().unwrap()[0].exit, None);
        shell.registry.close(&shell.id).await.unwrap();
        assert_eq!(std::env::var_os("SHELL"), before_shell);
        assert_eq!(std::env::current_dir().unwrap(), before_cwd);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_shell_detach_reattach_replays_and_preserves_process() {
        let shell = Shell::new().await;
        let (first, mut events) = shell.attach("local", false).await;
        assert!(
            shell
                .registry
                .input(&first.stream, "local", b"x".to_vec())
                .await
                .is_err(),
            "attach must not claim control"
        );
        assert!(
            shell
                .registry
                .control(&first.stream, "local", true)
                .unwrap()
        );
        shell
            .registry
            .resize(&first.stream, "local", 100, 30)
            .await
            .unwrap();
        let marker = format!("before-detach-{}-中文", Uuid::new_v4());
        shell
            .registry
            .input(&first.stream, "local", print_command(&marker))
            .await
            .unwrap();
        output_line(&mut events, &marker).await;
        shell.registry.detach(&first.stream, "local").unwrap();
        assert!(
            shell
                .registry
                .input(&first.stream, "local", b"x".to_vec())
                .await
                .is_err()
        );
        let (second, mut replay) = shell.attach("remote", false).await;
        assert_ne!(first.stream, second.stream);
        output_line(&mut replay, &marker).await;
        assert!(
            shell
                .registry
                .list()
                .unwrap()
                .iter()
                .any(|item| item.id == shell.id && item.exit.is_none())
        );
        assert!(
            shell
                .registry
                .control(&second.stream, "remote", true)
                .unwrap()
        );
        let after = format!("after-reattach-{}", Uuid::new_v4());
        shell
            .registry
            .input(&second.stream, "remote", print_command(&after))
            .await
            .unwrap();
        output_line(&mut replay, &after).await;
        shell.registry.close(&shell.id).await.unwrap();
        assert!(shell.registry.list().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn readonly_ownership_and_single_controller_are_enforced() {
        let shell = Shell::new().await;
        let (local, _local_events) = shell.attach("local", false).await;
        let (remote, _remote_events) = shell.attach("remote", false).await;
        let (reader, _reader_events) = shell.attach("viewer", true).await;
        assert!(
            shell
                .registry
                .control(&reader.stream, "viewer", true)
                .is_err()
        );
        assert!(
            shell
                .registry
                .resize(&reader.stream, "viewer", 90, 25)
                .await
                .is_err()
        );
        assert!(
            shell
                .registry
                .input(&reader.stream, "viewer", b"no".to_vec())
                .await
                .is_err()
        );
        assert!(
            shell
                .registry
                .control(&local.stream, "attacker", true)
                .is_err()
        );
        assert!(shell.registry.detach(&local.stream, "attacker").is_err());
        assert!(
            shell
                .registry
                .input(&local.stream, "attacker", b"no".to_vec())
                .await
                .is_err()
        );
        assert!(
            shell
                .registry
                .resize(&local.stream, "attacker", 90, 25)
                .await
                .is_err()
        );
        assert!(
            shell
                .registry
                .control(&local.stream, "local", true)
                .unwrap()
        );
        assert!(
            shell
                .registry
                .control(&remote.stream, "remote", true)
                .unwrap()
        );
        assert!(
            shell
                .registry
                .input(&local.stream, "local", b"no".to_vec())
                .await
                .is_err()
        );
        assert!(
            shell
                .registry
                .resize(&local.stream, "local", 90, 25)
                .await
                .is_err()
        );
        shell.registry.detach_all("remote");
        assert!(
            shell
                .registry
                .control(&remote.stream, "remote", true)
                .is_err()
        );
        assert!(
            shell
                .registry
                .control(&local.stream, "local", true)
                .unwrap()
        );
        shell.registry.close(&shell.id).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_exit_code_follows_final_output_and_is_replayed() {
        let shell = Shell::new().await;
        let (attachment, mut events) = shell.attach("local", false).await;
        shell
            .registry
            .control(&attachment.stream, "local", true)
            .unwrap();
        let marker = format!("final-output-{}", Uuid::new_v4());
        let mut command = print_command(&marker);
        #[cfg(unix)]
        command.extend_from_slice(b"exit 7\r");
        #[cfg(windows)]
        command.extend_from_slice(b"exit /b 7\r");
        shell
            .registry
            .input(&attachment.stream, "local", command)
            .await
            .unwrap();
        output_line(&mut events, &marker).await;
        let code = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Event::TerminalExit { code, .. } = events.recv().await.unwrap() {
                    break code;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(code, 7);
        assert_eq!(shell.registry.list().unwrap()[0].exit, Some(7));
        let (_, mut replay) = shell.attach("late-viewer", true).await;
        output_line(&mut replay, &marker).await;
        assert!(matches!(
            replay.recv().await,
            Some(Event::TerminalProgramStatus { .. })
        ));
        assert!(matches!(
            replay.recv().await,
            Some(Event::TerminalExit { code: 7, .. })
        ));
        shell.registry.close(&shell.id).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_subscriber_is_detached_without_blocking_pty() {
        let shell = Shell::new().await;
        let (events, _blocked) = mpsc::channel(1);
        let attachment = shell
            .registry
            .attach(&shell.id, "slow".into(), events, false)
            .await
            .unwrap();
        shell
            .registry
            .control(&attachment.stream, "slow", true)
            .unwrap();
        #[cfg(unix)]
        let flood = b"head -c 524288 /dev/zero; printf '\\n%s\\n' flood-finished\r".to_vec();
        #[cfg(windows)]
        let flood = b"for /L %i in (1,1,10000) do @echo flood-output\r".to_vec();
        shell
            .registry
            .input(&attachment.stream, "slow", flood)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if shell.registry.for_stream(&attachment.stream).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("slow attachment retained indefinitely");
        let terminal = shell.registry.terminal(&shell.id).unwrap();
        {
            let state = lock(&terminal.shared.state).unwrap();
            assert!(state.replay.len() <= REPLAY_BYTES);
            assert!(state.controller.is_none());
        }
        let (fresh, mut events) = shell.attach("fresh", false).await;
        shell
            .registry
            .control(&fresh.stream, "fresh", true)
            .unwrap();
        let marker = format!("after-flood-{}", Uuid::new_v4());
        shell
            .registry
            .input(&fresh.stream, "fresh", print_command(&marker))
            .await
            .unwrap();
        output_line(&mut events, &marker).await;
        shell.registry.close(&shell.id).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_event_receiver_releases_idle_attachment_and_control() {
        let shell = Shell::new().await;
        let (attachment, events) = shell.attach("gone", false).await;
        shell
            .registry
            .control(&attachment.stream, "gone", true)
            .unwrap();
        drop(events);
        tokio::time::timeout(Duration::from_secs(2), async {
            while shell.registry.for_stream(&attachment.stream).is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (next, _events) = shell.attach("next", false).await;
        assert!(shell.registry.control(&next.stream, "next", true).unwrap());
        shell.registry.close(&shell.id).await.unwrap();
    }

    #[test]
    fn replay_is_raw_bounded_and_announces_truncation() {
        let mut state = State::default();
        let bytes = vec![0xff_u8; REPLAY_BYTES + CHUNK];
        accept_output(&mut state, &bytes, |_| Ok(())).unwrap();
        assert_eq!(state.replay.len(), REPLAY_BYTES);
        assert!(state.truncated);
        assert!(state.replay.iter().all(|byte| *byte == 0xff));
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn explicit_close_terminates_hup_ignoring_owned_child() {
        let registry = TerminalRegistry::default();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args([
            "-c",
            "trap '' HUP; printf 'owned_hup_ready\\n'; exec sleep 30",
        ]);
        let id = registry
            .create_with_command("controlled HUP-ignoring child".into(), 80, 24, command)
            .await
            .unwrap();
        let shell = Shell { registry, id };
        let (attachment, mut events) = shell.attach("local", false).await;
        output_line(&mut events, "owned_hup_ready").await;
        shell
            .registry
            .close(&shell.id)
            .await
            .expect("explicit close must escalate beyond HUP and confirm owned-child exit");
        assert!(shell.registry.list().unwrap().is_empty());
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Event::TerminalExit { stream, .. } =
                    events.recv().await.expect("missing final owned-child exit")
                {
                    assert_eq!(stream, attachment.stream);
                    break;
                }
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn final_exit_is_drained_when_the_stop_sender_is_dropped_running() {
        let (events, incoming) = mpsc::channel(4);
        let (stop, stopped) = watch::channel(Stop::Running);
        let (outgoing, mut responses) = mpsc::channel(4);
        events
            .send(Event::TerminalExit {
                stream: "owned".into(),
                code: 7,
            })
            .await
            .unwrap();
        drop(events);
        drop(stop);
        deliver(
            DeliveryGuard {
                shared: Weak::new(),
                stream: "owned".into(),
            },
            "owned".into(),
            incoming,
            stopped,
            outgoing,
        )
        .await;
        assert!(matches!(
            responses.recv().await,
            Some(Event::TerminalExit { code: 7, .. })
        ));
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_pty_standard_queries_reply_once_resize_and_never_on_replay() {
        let da = replies::with(80, 24, |vt| vt.observe(b"\x1b[c")).unwrap();
        let hex = da
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let geometry = b"\x1b[8;12;40t";
        let geometry_hex = geometry
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let script = format!(
            r#"
stty raw -echo min 0 time 10
printf '\033['
sleep 0.02
printf 'c'
da=$(dd bs=1 count={da_len} 2>/dev/null | od -An -tx1 | tr -d '[:space:]')
[ "$da" = '{hex}' ] || exit 91
printf '\nVT_DA_OK\n'
stty min 1 time 0
[ "$(dd bs=1 count=1 2>/dev/null)" = r ] || exit 92
stty min 0 time 10
printf '\033[18t'
size=$(dd bs=1 count={geometry_len} 2>/dev/null | od -An -tx1 | tr -d '[:space:]')
[ "$size" = '{geometry_hex}' ] || exit 93
printf '\nVT_RESIZE_OK\n'
stty min 1 time 0
[ "$(dd bs=1 count=1 2>/dev/null)" = x ] || exit 94
stty min 0 time 2
extra=$(dd bs=1 count=1 2>/dev/null)
[ -z "$extra" ] || exit 95
printf '\nVT_NO_REPLAY_REPLY\n'
exit 7
"#,
            da_len = da.len(),
            geometry_len = geometry.len()
        );
        let registry = TerminalRegistry::default();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", &script]);
        let id = registry
            .create_with_command("real standard-query authority".into(), 80, 24, command)
            .await
            .unwrap();
        let shell = Shell { registry, id };
        let (first, mut output) = shell.attach("first", false).await;
        output_line(&mut output, "VT_DA_OK").await;
        // A second readonly observer sees replay but must never submit replies.
        let (second, _replayed) = shell.attach("readonly", true).await;
        assert!(
            shell
                .registry
                .control(&first.stream, "first", true)
                .unwrap()
        );
        shell
            .registry
            .resize(&first.stream, "first", 40, 12)
            .await
            .unwrap();
        shell
            .registry
            .input(&first.stream, "first", b"r".to_vec())
            .await
            .unwrap();
        output_line(&mut output, "VT_RESIZE_OK").await;
        shell.registry.detach(&first.stream, "first").unwrap();
        shell.registry.detach(&second.stream, "readonly").unwrap();
        let (third, mut output) = shell.attach("reattached", false).await;
        assert!(
            shell
                .registry
                .control(&third.stream, "reattached", true)
                .unwrap()
        );
        shell
            .registry
            .input(&third.stream, "reattached", b"x".to_vec())
            .await
            .unwrap();
        output_line(&mut output, "VT_NO_REPLAY_REPLY").await;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Event::TerminalExit { code, .. } = output.recv().await.unwrap() {
                    assert_eq!(code, 7);
                    break;
                }
            }
        })
        .await
        .unwrap();
        shell.registry.close(&shell.id).await.unwrap();
    }

    #[test]
    fn standard_reply_budget_failure_is_a_real_terminal_error_before_broadcast() {
        let pair = native_pty_system()
            .openpty(PtySize {
                cols: 80,
                rows: 24,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let (replies, reply_input) = blocking::sync_channel(IO_QUEUE);
        let authority = ReplyAuthority {
            master: Arc::new(Mutex::new(pair.master)),
            replies,
        };
        let (events, mut incoming) = mpsc::channel(8);
        let (stop, _stopped) = watch::channel(Stop::Running);
        let mut state = State {
            dimensions: (80, 24),
            ..State::default()
        };
        state.attachments.insert(
            "observer".into(),
            Subscriber {
                owner: "local".into(),
                read_only: true,
                events,
                stop,
                sequence: 0,
            },
        );
        let shared = Shared {
            state: Mutex::new(state),
            ..Shared::default()
        };
        let mut reader: Box<dyn Read + Send> =
            Box::new(std::io::Cursor::new(b"\x1b[c".repeat(5000)));
        let (output, chunks) = blocking::sync_channel(IO_QUEUE);
        read_output(&mut reader, output);
        let (_commands, commands) = blocking::sync_channel(IO_QUEUE);
        write_commands(
            Box::new(std::io::sink()),
            commands,
            chunks,
            reply_input,
            &shared,
            &authority,
        );
        assert!(matches!(incoming.try_recv(),Ok(Event::Error {code,..}) if code=="TERMINAL_ERROR"));
        assert!(
            incoming.try_recv().is_err(),
            "failed query chunk must not be broadcast"
        );
        let state = shared.state.lock().unwrap();
        assert!(state.read_done && state.failure.is_some());
        assert!(state.replay.is_empty());
    }
}
