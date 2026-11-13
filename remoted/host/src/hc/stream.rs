//! Each attachment owns one bounded connection and a cancellable delivery task.
//! hc is the VT authority; these bytes never enter the host PTY Replies/detector.
use super::{Attachment, Command, Hc, Record, SavedHc, model::programs};
use anyhow::{Context, Result, bail, ensure};
use futures_util::FutureExt;
use neko_hc_client::{CLOSE, CREDIT, Client, DATA, FOCUS, Frame, RESIZE};
use neko_remoted_protocol::{
    self as protocol,
    messages::{Event, ProgramRecord},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

struct Delivery {
    events: Vec<Event>,
    output: Option<Vec<u8>>,
    credit: Option<(u32, u32)>,
    _budget: OwnedSemaphorePermit,
}
struct Delivered {
    credit: Option<(u32, u32)>,
    _budget: OwnedSemaphorePermit,
}
struct Output {
    stream: String,
    queue: mpsc::Sender<Delivery>,
    budget: Arc<Semaphore>,
}
impl Output {
    fn push(
        &self,
        events: Vec<Event>,
        output: Option<Vec<u8>>,
        credit: Option<(u32, u32)>,
    ) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Response<'a> {
            id: u64,
            event: &'a Event,
        }
        for event in &events {
            // A core status table exceeding the wire limit retires only its stream.
            protocol::pack(&Response { id: 0, event })?;
        }
        let size = output.as_ref().map_or(0, Vec::len) + serde_json::to_vec(&events)?.len();
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(u32::try_from(size.max(1))?)
            .context("hc consumer exceeded its byte budget; reattach the same terminal")?;
        self.queue
            .try_send(Delivery {
                events,
                output,
                credit,
                _budget: permit,
            })
            .context("hc consumer queue full or stopped; reattach the same terminal")
    }
    fn status(&self, records: Vec<ProgramRecord>) -> Result<()> {
        self.push(
            vec![Event::TerminalProgramStatus {
                stream: self.stream.clone(),
                records,
            }],
            None,
            None,
        )
    }
}
impl Hc {
    /// A completed attach remains a valid stream identity even if its reader
    /// fails before local focus can be confirmed. Retire it through the normal
    /// stream event after TerminalAttached, never as a fabricated controller.
    pub async fn acquire_on_attach(&self, stream: &str, subscriber: &str) {
        if let Err(error) = self.control(stream, subscriber, true).await {
            let attachment = {
                let mut attachments = self.attachments.lock().expect("hc attachments lock");
                if attachments
                    .get(stream)
                    .is_some_and(|item| item.subscriber == subscriber)
                {
                    attachments.remove(stream)
                } else {
                    None
                }
            };
            if let Some(attachment) = attachment {
                attachment.cancel.cancel();
                let event = unavailable(
                    stream,
                    format!(
                        "hc initial control failed: {error}; reattach the same resource, do not create another terminal"
                    ),
                );
                // Reader retirement may already be queued. Session ownership
                // drops any duplicate after the first final stream event.
                tokio::spawn(async move {
                    let _ = notify(&attachment.events, event).await;
                });
            }
        }
    }

    pub async fn attach(
        self: &Arc<Self>,
        grant: &str,
        subscriber: String,
        events: mpsc::Sender<Event>,
        read_only: bool,
    ) -> Result<(String, String)> {
        ensure!(
            self.attachments.lock().expect("hc attachments lock").len() < 16,
            "hc attachment limit reached"
        );
        let (mut client, saved, record, terminal) = self.terminal(grant).await?;
        let stream = uuid::Uuid::new_v4().to_string();
        let cancel = self.shutdown.child_token();
        let (commands, rx) = mpsc::channel(8);
        let (queue, delivery) = mpsc::channel(64);
        let (consumed, credits) = mpsc::channel(64);
        let output = Output {
            stream: stream.clone(),
            queue,
            budget: Arc::new(Semaphore::new(2 * 1024 * 1024)),
        };
        let delivery_cancel = cancel.clone();
        let delivery_events = events.clone();
        let delivery_stream = stream.clone();
        // Delivery never holds the hc reader or command worker behind a slow downstream queue.
        let mut delivery_task = tokio::spawn(async move {
            let result = deliver(
                delivery_stream,
                delivery_events,
                delivery,
                consumed,
                &delivery_cancel,
            )
            .await;
            if result.is_err() {
                delivery_cancel.cancel();
            }
            result
        });
        let setup = async {
            if terminal["state"] == "finished" {
                let screen = client.call("terminal.read", json!({"terminal":terminal["address"],"lines":1000})).await?;
                let text = screen["text"].as_str().context("hc omitted final screen")?.replace('\n', "\r\n");
                output.push(Vec::new(), Some(text.into_bytes()), None)?;
                output.status(Vec::new())?;
                output.push(vec![ended(&stream, &terminal)], None, None)?;
                return Ok(None);
            }
            let attach = client.call("terminal.attach", json!({"terminal":terminal["address"],"cols":80,"rows":24,"history":1000,"observe":read_only,"resumed":true,"display":{}})).await?;
            let channel = stream_channel(&attach)?;
            if let Some(attached) = attach.get("terminal") { saved.verify(&record, attached)?; }
            let snapshot = client.call("events.subscribe", json!({"terminal":terminal["address"],"labels":saved.labels(&record)})).await?;
            let events_channel = stream_channel(&snapshot)?;
            ensure!(channel != events_channel, "hc reused a live channel");
            let table = snapshot_records(&saved, &record, &snapshot)?;
            let seq = snapshot["seq"].as_u64().context("hc snapshot omitted sequence")?;
            Ok(Some((channel, events_channel, table, seq)))
        }.await;
        let setup = match setup {
            Ok(setup) => setup,
            Err(error) => {
                cancel.cancel();
                let _ = delivery_task.await;
                return Err(error);
            }
        };
        {
            let mut attachments = self.attachments.lock().expect("hc attachments lock");
            if attachments.len() >= 16 {
                cancel.cancel();
                delivery_task.abort();
                bail!("hc attachment limit reached");
            }
            attachments.insert(
                stream.clone(),
                Attachment {
                    grant: grant.into(),
                    subscriber: subscriber.clone(),
                    read_only,
                    controller: false,
                    commands,
                    events: events.clone(),
                    cancel: cancel.clone(),
                },
            );
        }
        let name = record.name.clone();
        let host = self.clone();
        let id = stream.clone();
        tokio::spawn(async move {
            let result = if let Some((channel, events_channel, records, sequence)) = setup {
                let mut state = Stream {
                    host: &host,
                    subscriber: &subscriber,
                    output: &output,
                    terminal: channel,
                    events: events_channel,
                    saved: &saved,
                    record: &record,
                    records,
                    sequence,
                    partial: Vec::new(),
                    live: false,
                };
                // attach's first paint can precede the RPC replies. Drain already available
                // paints before publishing the current status; core has no paint-end marker.
                let initial = async {
                    for _ in 0..64 {
                        let Some(frame) = client.next_frame().now_or_never() else {
                            break;
                        };
                        let frame = frame?.context("hc connection lost during attach")?;
                        if !state.frame(frame)? {
                            return Ok(false);
                        }
                    }
                    state.output.status(state.records.clone())?;
                    state.live = true;
                    Ok(true)
                }
                .await;
                match initial {
                    Ok(true) => run(&mut client, &mut state, rx, credits, &cancel).await,
                    Ok(false) => Ok(()),
                    Err(error) => Err(error),
                }
            } else {
                Ok(())
            };
            let mut failure = result
                .err()
                .map(|error| format!("hc attachment ended: {error}"));
            drop(output);
            // Drain queued output before retiring this stream. On overflow or delivery
            // failure, the final notification must not depend on that full queue.
            let drained = tokio::select! {
                biased;
                result = tokio::time::timeout(Duration::from_secs(3), &mut delivery_task) => Some(result),
                () = cancel.cancelled() => None,
            };
            let incomplete = match drained {
                Some(Ok(Ok(Ok(())))) | None => false,
                Some(Ok(Ok(Err(error)))) => {
                    failure.get_or_insert_with(|| format!("hc output delivery failed: {error}"));
                    true
                }
                Some(Ok(Err(error))) => {
                    failure.get_or_insert_with(|| format!("hc output delivery stopped: {error}"));
                    true
                }
                Some(Err(_)) => {
                    failure.get_or_insert_with(|| "hc output delivery timed out".into());
                    true
                }
            };
            delivery_task.abort();
            if let Some(mut message) = failure {
                if incomplete {
                    message.push_str("; queued output may be incomplete");
                }
                message.push_str("; reattach the same resource, do not create another terminal");
                // A temporarily full event queue must not silently discard the only
                // terminal state. A receiver that never drains remains bounded too.
                let _ = notify(&events, unavailable(&id, message)).await;
            }
            cancel.cancel();
            // Closing only this connection/attachment must never kill core's PTY.
            let _ = client.close().await;
            host.attachments
                .lock()
                .expect("hc attachments lock")
                .remove(&id);
        });
        Ok((stream, name))
    }
}
async fn deliver(
    stream: String,
    events: mpsc::Sender<Event>,
    mut queue: mpsc::Receiver<Delivery>,
    consumed: mpsc::Sender<Delivered>,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut sequence = 0_u64;
    let mut first = true;
    loop {
        let delivery = tokio::select! { biased; () = cancel.cancelled() => return Ok(()), item = queue.recv() => item };
        let Some(delivery) = delivery else {
            return Ok(());
        };
        if let Some(bytes) = delivery.output {
            for chunk in bytes.chunks(protocol::FILE_CHUNK) {
                sequence = sequence
                    .checked_add(1)
                    .context("hc output sequence exhausted")?;
                let event = Event::TerminalOutput {
                    stream: stream.clone(),
                    sequence,
                    data: protocol::encode(chunk),
                    replay: first,
                    reset: first,
                    gap: false,
                };
                tokio::select! { biased; () = cancel.cancelled() => return Ok(()), result = events.send(event) => result? }
                first = false;
            }
        }
        for event in delivery.events {
            tokio::select! { biased; () = cancel.cancelled() => return Ok(()), result = events.send(event) => result? }
        }
        // Application queue consumption, not Relay ACK or a fabricated renderer receipt.
        if delivery.credit.is_some() {
            tokio::select! { biased; () = cancel.cancelled() => return Ok(()), _ = consumed.send(Delivered { credit: delivery.credit, _budget: delivery._budget }) => {} }
        }
    }
}
struct Stream<'a> {
    host: &'a Hc,
    subscriber: &'a str,
    output: &'a Output,
    terminal: u32,
    events: u32,
    saved: &'a SavedHc,
    record: &'a Record,
    records: Vec<ProgramRecord>,
    sequence: u64,
    partial: Vec<u8>,
    live: bool,
}
impl Stream<'_> {
    fn frame(&mut self, frame: Frame) -> Result<bool> {
        if frame.channel == 0 {
            return Ok(true);
        }
        ensure!(
            frame.channel == self.terminal || frame.channel == self.events,
            "hc sent data on an unowned channel"
        );
        if frame.kind == CLOSE {
            if frame.channel == self.events {
                bail!("hc events ended; reattach for a fresh snapshot");
            }
            let reason: Value =
                serde_json::from_slice(&frame.payload).context("invalid hc close reason")?;
            self.output.status(Vec::new())?;
            self.output
                .push(vec![ended(&self.output.stream, &reason)], None, None)?;
            return Ok(false);
        }
        ensure!(frame.kind == DATA, "unexpected hc output frame kind");
        let credit = Some((frame.channel, u32::try_from(frame.payload.len())?));
        if frame.channel == self.terminal {
            self.output.push(Vec::new(), Some(frame.payload), credit)?;
        } else {
            let mut changed = false;
            let mut count = 0;
            for part in frame.payload.split_inclusive(|b| *b == b'\n') {
                ensure!(
                    self.partial.len() + part.len() <= 1024 * 1024 && count < 1024,
                    "hc event line budget exceeded"
                );
                self.partial.extend_from_slice(part);
                if !part.ends_with(b"\n") {
                    continue;
                }
                count += 1;
                let line: Value = serde_json::from_slice(&self.partial)?;
                self.partial.clear();
                let params = &line["params"];
                let seq = params["seq"]
                    .as_u64()
                    .context("hc event omitted sequence")?;
                if seq < self.sequence {
                    continue;
                }
                let next = match line["method"].as_str() {
                    Some("missed") => Some(snapshot_records(self.saved, self.record, params)?),
                    Some("event") if seq > self.sequence => {
                        ensure!(
                            params["terminal"].as_str() == self.record.address.as_deref(),
                            "hc event belongs to another terminal"
                        );
                        if params["type"] == "status" {
                            Some(programs(&params["data"]))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                self.sequence = seq;
                if let Some(next) = next
                    && self.records != next
                {
                    self.records = next;
                    changed = true;
                }
            }
            let events = if changed && self.live {
                vec![Event::TerminalProgramStatus {
                    stream: self.output.stream.clone(),
                    records: self.records.clone(),
                }]
            } else {
                Vec::new()
            };
            self.output.push(events, None, credit)?;
        }
        Ok(true)
    }
}
async fn run(
    client: &mut Client,
    state: &mut Stream<'_>,
    mut commands: mpsc::Receiver<Command>,
    mut credits: mpsc::Receiver<Delivered>,
    cancel: &CancellationToken,
) -> Result<()> {
    loop {
        tokio::select! { biased;
            () = cancel.cancelled() => return Ok(()),
            Some(command) = commands.recv() => {
                let (kind, bytes, done, permitted) = match command {
                    Command::Input(bytes, done) => (DATA, bytes, Some(done), state.host.controlled(&state.output.stream, state.subscriber)),
                    Command::Resize(cols, rows, done) => { let mut bytes = cols.to_le_bytes().to_vec(); bytes.extend_from_slice(&rows.to_le_bytes()); (RESIZE, bytes, Some(done), state.host.controlled(&state.output.stream, state.subscriber)) },
                    Command::Focus(focus, done) => (FOCUS, vec![u8::from(focus)], done, !focus || state.host.controlled(&state.output.stream, state.subscriber)),
                };
                let result = if permitted { client.send(state.terminal, kind, &bytes).await } else { Err(anyhow::anyhow!("hc control changed before the operation was sent")) };
                let failed = permitted && result.is_err();
                if let Some(done) = done { let _ = done.send(result); }
                if failed { bail!("hc write failed; operation outcome is unknown and must not be replayed"); }
            },
            Some(delivered) = credits.recv() => {
                if let Some((channel, bytes)) = delivered.credit { client.send(channel, CREDIT, &bytes.to_le_bytes()).await?; }
            },
            frame = client.next_frame() => {
                let frame = frame?.context("hc connection lost; its terminal may still be running")?;
                if !state.frame(frame)? { return Ok(()); }
            }
        }
    }
}
fn stream_channel(value: &Value) -> Result<u32> {
    value["channel"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n != 0)
        .context("hc omitted a nonzero stream channel")
}
fn snapshot_records(
    saved: &SavedHc,
    record: &Record,
    snapshot: &Value,
) -> Result<Vec<ProgramRecord>> {
    let list = snapshot["terminals"]
        .as_array()
        .context("hc omitted status snapshot")?;
    ensure!(
        list.len() == 1,
        "hc subscription snapshot is not scoped to its granted terminal"
    );
    saved.verify(record, &list[0])?;
    Ok(programs(&list[0]))
}
pub(super) fn ended(stream: &str, value: &Value) -> Event {
    let reason = value["reason"].as_str().unwrap_or("unknown");
    if matches!(reason, "exited" | "killed" | "finished") {
        if let Some(code) = value["exit_code"]
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
        {
            return Event::TerminalExit {
                stream: stream.into(),
                code,
            };
        }
        unavailable(
            stream,
            format!("hc terminal ended ({reason}); no numeric exit status was supplied"),
        )
    } else {
        unavailable(
            stream,
            format!(
                "hc channel closed ({reason}); reattach the same resource; the program may still be running"
            ),
        )
    }
}

fn unavailable(stream: &str, message: String) -> Event {
    Event::TerminalUnavailable {
        stream: stream.into(),
        // Untrusted core diagnostics must fit a business response, even after JSON escaping.
        message: message
            .chars()
            .filter(|c| !c.is_control())
            .take(2048)
            .collect(),
    }
}

pub(super) async fn notify(events: &mpsc::Sender<Event>, event: Event) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(3), events.send(event))
        .await
        .context("hc final notification timed out")?
        .context("hc final notification receiver closed")
}
