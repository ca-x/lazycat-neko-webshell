use crate::state::{Core, DEADLINE, Device, State};
use anyhow::{Context, Result, bail, ensure};
use neko_remoted_protocol::{
    self as protocol,
    messages::{Event, Operation, Permission, Request, Resource, Response},
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) enum Principal {
    Local,
    Device { id: String, generation: u64 },
}
pub(crate) fn authorize<'a>(state: &'a State, principal: &Principal) -> Result<Option<&'a Device>> {
    match principal {
        Principal::Local => Ok(None),
        Principal::Device { id, generation } => {
            ensure!(state.saved.enabled, "hosting disabled");
            let device = state
                .saved
                .devices
                .iter()
                .find(|d| d.id == *id && d.generation == *generation)
                .context("device revoked or permissions changed")?;
            Ok(Some(device))
        }
    }
}
pub(crate) fn resources(core: &Core, device: Option<&Device>) -> Result<Vec<Resource>> {
    Ok(core
        .resources()?
        .into_iter()
        .filter(|r| device.is_none_or(|d| d.resources.contains(&r.id)))
        .collect())
}
struct Detach {
    core: Arc<Core>,
    subscriber: String,
}
impl Drop for Detach {
    fn drop(&mut self) {
        self.core.terminals.detach_all(&self.subscriber);
        self.core.herdr.detach_all(&self.subscriber);
        self.core.hc.detach_all(&self.subscriber);
    }
}
fn send(tx: &mpsc::Sender<Response>, id: u64, event: Event) -> Result<()> {
    let response = Response { id, event };
    protocol::pack(&response)?;
    tx.try_send(response)
        .context("business response queue full or closed")
}
pub(crate) async fn serve(
    core: Arc<Core>,
    principal: Principal,
    subscriber: String,
    mut requests: mpsc::Receiver<Request>,
    responses: mpsc::Sender<Response>,
    cancel: CancellationToken,
) {
    let _detach = Detach {
        core: core.clone(),
        subscriber: subscriber.clone(),
    };
    let result = serve_inner(
        &core,
        &principal,
        &subscriber,
        &mut requests,
        &responses,
        &cancel,
    )
    .await;
    if result.is_err() {
        cancel.cancel();
    }
}
async fn serve_inner(
    core: &Core,
    principal: &Principal,
    subscriber: &str,
    requests: &mut mpsc::Receiver<Request>,
    responses: &mpsc::Sender<Response>,
    cancel: &CancellationToken,
) -> Result<()> {
    if !matches!(principal, Principal::Local) {
        let state = core.state.lock().await;
        let device = authorize(&state, principal)?;
        send(
            responses,
            0,
            Event::Ready {
                device_id: device.map_or("local", |d| d.id.as_str()).into(),
                permission: device.map_or(Permission::Operate, |d| d.permission),
                resources: resources(core, device)?,
            },
        )?;
    }
    let (events, mut incoming) = mpsc::channel(32);
    let mut streams = HashMap::<String, StreamGrant>::new();
    let mut last = 0;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return Ok(()),
            request = requests.recv() => {
                let request = request.context("business input closed")?;
                ensure!(request.id > last && request.id > 0, "request ID replay");
                last = request.id;
                let outcome_unknown = matches!(request.operation, Operation::TerminalInput { .. } | Operation::TerminalResize { .. } | Operation::Herdr { .. });
                let state = core.state.lock().await;
                ensure!(!cancel.is_cancelled(), "session cancelled");
                let device = authorize(&state, principal)?;
                let can_write = device.is_none_or(|d| d.permission.can_write());
                let allowed = resources(core, device)?;
                let result = tokio::time::timeout(DEADLINE,
                    dispatch(core, subscriber, Access { write: can_write, local: matches!(principal, Principal::Local) }, &allowed, &mut streams, &events, request.operation))
                    .await.map_err(|_| anyhow::anyhow!("operation timed out; do not replay mutations"))
                    .and_then(|result| result);
                core.publish(&state)?;
                // Do not dequeue replay/live output until the attached response is queued.
                let event = result.unwrap_or_else(|_| Event::Error { code: "REQUEST_FAILED".into(),
                    message: "Operation unavailable, invalid or not authorized".into(), outcome_unknown });
                send(responses, request.id, event)?;
            }
            event = incoming.recv() => {
                let event = event.context("terminal event stream closed")?;
                let state = core.state.lock().await;
                authorize(&state, principal)?;
                ensure!(!cancel.is_cancelled(), "session cancelled");
                let stream = match &event {
                    Event::TerminalOutput { stream, .. } | Event::TerminalControl { stream, .. }
                        | Event::TerminalExit { stream, .. } | Event::TerminalUnavailable { stream, .. }
                        | Event::TerminalProgramStatus { stream, .. } => Some(stream),
                    _ => None,
                };
                if let Some(stream) = stream {
                    if matches!(&event, Event::TerminalExit { .. } | Event::TerminalUnavailable { .. }) {
                        // Closing can withdraw the resource before its final event arrives.
                        // Only terminal retirement may cross that withdrawal; the stream
                        // must still belong to this authenticated session.
                        if streams.remove(stream).is_none() { continue; }
                    } else if check_stream(stream, &streams, &resources(core, authorize(&state, principal)?)?).is_err() {
                        continue;
                    }
                }
                send(responses, 0, event)?;
            }

        }
    }
}
struct StreamGrant {
    resource: String,
    pane: Option<String>,
}
struct Access {
    write: bool,
    local: bool,
}

async fn dispatch(
    core: &Core,
    subscriber: &str,
    access: Access,
    resources: &[Resource],
    streams: &mut HashMap<String, StreamGrant>,
    events: &mpsc::Sender<Event>,
    operation: Operation,
) -> Result<Event> {
    let write = access.write;
    match operation {
        Operation::Ping { nonce } => {
            ensure!(nonce.len() <= 128, "heartbeat nonce exceeds limit");
            Ok(Event::Pong { nonce })
        }
        Operation::Resources {} => Ok(Event::Resources {
            resources: resources.to_vec(),
        }),
        Operation::TerminalAttach { grant, pane } => {
            let resource = resources
                .iter()
                .find(|r| r.id == grant)
                .context("resource is not granted")?;
            ensure!(
                resource.terminal || resource.herdr,
                "resource has no terminal capability"
            );
            ensure!(
                streams.len() < 8
                    && !streams
                        .values()
                        .any(|item| item.resource == grant && item.pane == pane),
                "terminal already attached or stream limit reached"
            );
            if resource.backend == "hc" {
                ensure!(pane.is_none(), "hc terminal cannot select a Herdr pane");
                let (stream, name) = core
                    .hc
                    .attach(&grant, subscriber.to_owned(), events.clone(), !write)
                    .await?;
                streams.insert(
                    stream.clone(),
                    StreamGrant {
                        resource: grant,
                        pane: None,
                    },
                );
                return Ok(Event::TerminalAttached {
                    stream,
                    name,
                    read_only: !write,
                });
            }
            if resource.herdr {
                let (stream, name) = core
                    .herdr
                    .attach(
                        pane.clone().context("select a Herdr pane")?,
                        subscriber.to_owned(),
                        events.clone(),
                        !write,
                    )
                    .await?;
                streams.insert(
                    stream.clone(),
                    StreamGrant {
                        resource: grant,
                        pane,
                    },
                );
                return Ok(Event::TerminalAttached {
                    stream,
                    name,
                    read_only: !write,
                });
            }
            ensure!(pane.is_none(), "ordinary terminal cannot select a pane");
            let attachment = core
                .terminals
                .attach(&grant, subscriber.to_owned(), events.clone(), !write)
                .await?;
            streams.insert(
                attachment.stream.clone(),
                StreamGrant {
                    resource: grant,
                    pane: None,
                },
            );
            Ok(Event::TerminalAttached {
                stream: attachment.stream,
                name: attachment.name,
                read_only: attachment.read_only,
            })
        }
        Operation::TerminalControl { stream, take } => {
            check_stream(&stream, streams, resources)?;
            ensure!(!take || write, "read-only device");
            let controller = if core.herdr.is_stream(&stream, subscriber) {
                core.herdr.control(&stream, subscriber, take)?
            } else if core.hc.is_stream(&stream, subscriber) {
                core.hc.control(&stream, subscriber, take).await?
            } else {
                core.terminals.control(&stream, subscriber, take)?
            };
            Ok(Event::TerminalControl { stream, controller })
        }
        Operation::TerminalInput { stream, data } => {
            check_stream(&stream, streams, resources)?;
            ensure!(write, "read-only device");
            let data = protocol::decode(&data, 16_384)?;
            if core.herdr.is_stream(&stream, subscriber) {
                core.herdr.input(&stream, subscriber, data).await?;
            } else if core.hc.is_stream(&stream, subscriber) {
                core.hc.input(&stream, subscriber, data).await?;
            } else {
                core.terminals.input(&stream, subscriber, data).await?;
            }
            Ok(Event::Ok)
        }
        Operation::TerminalResize { stream, cols, rows } => {
            check_stream(&stream, streams, resources)?;
            ensure!(
                write && (2..=1000).contains(&cols) && (2..=1000).contains(&rows),
                "invalid resize or read-only device"
            );
            ensure!(
                !core.herdr.is_stream(&stream, subscriber),
                "Herdr pane size belongs to its layout; terminal resize is unsupported"
            );
            if core.hc.is_stream(&stream, subscriber) {
                core.hc.resize(&stream, subscriber, cols, rows).await?;
            } else {
                core.terminals
                    .resize(&stream, subscriber, cols, rows)
                    .await?;
            }
            Ok(Event::Ok)
        }
        Operation::TerminalDetach { stream } => {
            check_stream(&stream, streams, resources)?;
            if core.herdr.is_stream(&stream, subscriber) {
                core.herdr.detach(&stream, subscriber)?;
            } else if core.hc.is_stream(&stream, subscriber) {
                core.hc.detach(&stream, subscriber)?;
            } else {
                core.terminals.detach(&stream, subscriber)?;
            }
            streams.remove(&stream);
            Ok(Event::Ok)
        }
        Operation::Herdr {
            grant,
            method,
            params,
        } => {
            ensure!(
                resources.iter().any(|r| r.id == grant && r.herdr),
                "Herdr resource is not granted"
            );
            ensure!(
                access.local
                    || protocol::messages::herdr_allowed(
                        if write {
                            Permission::Operate
                        } else {
                            Permission::ReadOnly
                        },
                        &method
                    ),
                "Herdr method is not allowed for this device"
            );
            if matches!(
                method.as_str(),
                "pane.send_text" | "pane.send_keys" | "pane.send_input" | "pane.resize"
            ) {
                ensure!(write, "read-only device");
                core.herdr.require_control(
                    params["pane_id"].as_str().context("pane_id required")?,
                    subscriber,
                )?;
            }
            let data = core.herdr.request(&method, params).await?;
            protocol::pack(&Response {
                id: u64::MAX,
                event: Event::Herdr { data: data.clone() },
            })?;
            Ok(Event::Herdr { data })
        }
        // No remote shell creation, filesystem paths or caller-selected Herdr endpoints.
        _ => bail!("unsupported operation"),
    }
}
fn check_stream(
    stream: &str,
    streams: &HashMap<String, StreamGrant>,
    resources: &[Resource],
) -> Result<()> {
    ensure!(
        streams
            .get(stream)
            .is_some_and(|grant| resources.iter().any(|r| r.id == grant.resource)),
        "stream is not authorized"
    );
    Ok(())
}
