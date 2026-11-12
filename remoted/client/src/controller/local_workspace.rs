//! Desktop host lifecycle and direct local workspace access.
use super::*;
use crate::platform::HOST_IDENTITY_KEY;
use protocol::messages::SessionBackend;

impl Actor {
    pub(super) async fn initialize_host(&mut self) -> Result<()> {
        let saved = crate::platform::secure_get(HOST_IDENTITY_KEY).await?;
        let save: neoko_terminal_host::Save = Arc::new(|value| {
            Box::pin(async move { crate::platform::secure_set(HOST_IDENTITY_KEY, &value).await })
        });
        let host = neoko_terminal_host::start(saved.as_deref(), save).await?;
        self.model.host = Some(host.snapshot());
        self.host_updates = Some(host.subscribe());
        self.host = Some(host);
        Ok(())
    }

    pub(super) fn refresh_host(&mut self) {
        if let Some(host) = &self.host {
            let snapshot = host.snapshot();
            if self.model.local && self.model.ready() {
                self.model.resources = snapshot.resources.clone();
            }
            self.model.host = Some(snapshot);
        }
    }

    pub(super) async fn select_local(&mut self, persist: bool) -> Result<()> {
        let host = self.host.clone().context("本机终端服务尚未就绪")?;
        self.stop_retry();
        self.disconnected().await;
        self.clear_workspace();
        self.model.active_binding = None;
        self.model.local = true;
        self.model.phase = Phase::Connecting;
        self.publish().await;
        let scope = self.scope.clone();
        let generation = self.model.generation;
        let local = scope
            .during(generation, async {
                if persist {
                    storage::select_local_workspace().await?;
                }
                host.local().await
            })
            .await?;
        self.session = Some(WorkspaceChannel::Local(local));
        self.model.phase = Phase::Ready;
        self.model.permission = Some(Permission::Operate);
        self.model.fingerprint.clear();
        self.model.notice.clear();
        self.model.error.clear();
        self.refresh_host();
        Ok(())
    }

    pub(super) async fn create_local_session(&mut self, backend: SessionBackend) -> Result<()> {
        ensure!(self.model.local && self.model.ready(), "请先选择本机工作区");
        let host = self.host.clone().context("本机终端服务不可用")?;
        let generation = self.model.generation;
        let scope = self.scope.clone();
        let name = match backend {
            SessionBackend::Native => "终端",
            SessionBackend::Hc => "hc 终端",
            SessionBackend::Herdr => "Herdr",
        };
        let created = scope
            .during(generation, host.create_session(backend, name.into()))
            .await?;
        self.refresh_host();
        if created.pane.is_some() {
            self.request(
                Operation::Herdr {
                    grant: created.grant.clone(),
                    method: "session.snapshot".into(),
                    params: serde_json::json!({}),
                },
                Pending::Herdr {
                    grant: created.grant.clone(),
                    method: "session.snapshot".into(),
                },
            )
            .await?;
        }
        let attachment = Attachment {
            grant: created.grant,
            pane: created.pane,
        };
        self.request(
            Operation::TerminalAttach {
                grant: attachment.grant.clone(),
                pane: attachment.pane.clone(),
            },
            Pending::Attach(attachment),
        )
        .await
    }

    pub(super) async fn close_local_terminal(&mut self, id: &str) -> Result<()> {
        ensure!(
            self.model.local && self.model.ready(),
            "只能结束当前本机工作区的会话"
        );
        ensure!(
            self.model
                .resources
                .iter()
                .any(|r| r.id == id && r.terminal && !r.herdr),
            "本机会话已变化，请刷新后重试"
        );
        let host = self.host.clone().context("本机终端服务不可用")?;
        let scope = self.scope.clone();
        scope
            .during(self.model.generation, host.close_terminal(id))
            .await?;
        // An explicit successful close ends this resource. Its final output may
        // race resource withdrawal; do not leave writable stale panes behind.
        let streams: Vec<String> = self
            .attachments
            .iter()
            .filter(|(_, attachment)| attachment.grant == id)
            .map(|(stream, _)| stream.clone())
            .collect();
        for stream in streams {
            self.retire_stream(&stream);
        }
        for pending in self.outstanding.values_mut() {
            if matches!(&pending.kind, Pending::Attach(attachment) | Pending::Restore { attachment, .. } if attachment.grant == id)
            {
                pending.kind = Pending::RetiredAttach;
            }
        }
        self.refresh_host();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn attach_request(actor: &mut Actor, grant: &str) -> u64 {
        actor
            .request(
                Operation::TerminalAttach {
                    grant: grant.into(),
                    pane: None,
                },
                Pending::Attach(Attachment {
                    grant: grant.into(),
                    pane: None,
                }),
            )
            .await
            .unwrap();
        *actor.outstanding.keys().max().unwrap()
    }

    async fn actual_reply(actor: &mut Actor, id: u64) -> Response {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let response = actor.session.as_mut().unwrap().receive().await.unwrap();
                if response.id == id {
                    return response;
                }
                actor.response(response).await.unwrap();
            }
        })
        .await
        .expect("real local-session reply timed out")
    }

    // Only the credential sink is in memory. Requests, replies, registry,
    // foreground shell PTYs, close, replay and actor response handling are real.
    async fn real_local_close(hold_attach_reply: bool) {
        let saved = Arc::new(std::sync::Mutex::new(None::<String>));
        let sink = saved.clone();
        let host = neoko_terminal_host::start(
            None,
            Arc::new(move |value| {
                let sink = sink.clone();
                Box::pin(async move {
                    *sink.lock().unwrap() = Some(value);
                    Ok(())
                })
            }),
        )
        .await
        .unwrap();
        assert!(saved.lock().unwrap().is_some());
        let (updates, _incoming_updates) = mpsc::channel(1024);
        let mut actor = Actor::new(updates);
        actor.host = Some(host.clone());
        actor.select_local(false).await.unwrap();
        let survivor = host
            .create_terminal("close-test-survivor".into())
            .await
            .unwrap();
        let closing = host
            .create_terminal("close-test-target".into())
            .await
            .unwrap();
        actor.refresh_host();

        let id = attach_request(&mut actor, &survivor).await;
        let reply = actual_reply(&mut actor, id).await;
        assert!(matches!(&reply.event, Event::TerminalAttached { .. }));
        actor.response(reply).await.unwrap();
        let survivor_stream = actor.model.panes[0].stream.clone();

        let id = attach_request(&mut actor, &closing).await;
        // Hold a genuine attached reply at the transport/actor seam. The host
        // has installed the stream, but the actor has not observed it yet.
        let reply = actual_reply(&mut actor, id).await;
        let Event::TerminalAttached { stream, .. } = &reply.event else {
            panic!("real PTY attach failed: {:?}", reply.event);
        };
        let closing_stream = stream.clone();
        let held_reply = if hold_attach_reply {
            Some(reply)
        } else {
            actor.response(reply).await.unwrap();
            None
        };
        actor.close_local_terminal(&closing).await.unwrap();
        let pending_after_close = actor.outstanding.values().any(|request| {
            matches!(&request.kind,
                Pending::Attach(attachment) | Pending::Restore { attachment, .. }
                if attachment.grant == closing)
        });
        let mut late_errors = Vec::new();
        if let Some(reply) = held_reply
            && let Err(error) = actor.response(reply).await
        {
            late_errors.push(error.to_string());
        }
        // Replay/exit can already be queued behind the held reply. Closing a
        // resource must make those stale stream events harmless as well.
        for _ in 0..64 {
            let next = tokio::time::timeout(
                Duration::from_millis(100),
                actor.session.as_mut().unwrap().receive(),
            )
            .await;
            match next {
                Ok(Ok(response)) => {
                    if let Err(error) = actor.response(response).await {
                        late_errors.push(error.to_string());
                    }
                }
                Err(_) => break,
                Ok(Err(error)) => {
                    late_errors.push(error.to_string());
                    break;
                }
            }
        }
        let stale_pane = actor
            .model
            .panes
            .iter()
            .any(|pane| pane.stream == closing_stream);
        let stale_attachment = actor.attachments.contains_key(&closing_stream);
        let stale_output = actor.output.contains_key(&closing_stream);
        let target_removed = !host.snapshot().resources.iter().any(|r| r.id == closing);
        let survivor_retained = actor.model.ready()
            && actor.model.resources.iter().any(|r| r.id == survivor)
            && actor
                .model
                .panes
                .iter()
                .any(|pane| pane.stream == survivor_stream);

        host.close_terminal(&survivor).await.unwrap();
        actor.disconnected().await;
        assert!(
            target_removed && survivor_retained,
            "close affected the wrong local resource"
        );
        assert!(
            !pending_after_close
                && !stale_pane
                && !stale_attachment
                && !stale_output
                && late_errors.is_empty(),
            "closed local PTY resurrected or late events failed: hold_attach_reply={hold_attach_reply}, pending={pending_after_close}, pane={stale_pane}, attachment={stale_attachment}, output={stale_output}, errors={late_errors:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_local_session_close_cleans_confirmed_attachment() {
        real_local_close(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_local_session_close_ignores_pending_attached_reply() {
        real_local_close(true).await;
    }
}
