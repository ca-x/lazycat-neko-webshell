//! Volatile composer submissions. A transport acknowledgment accepts input, not execution.
use super::*;
use std::collections::VecDeque;

pub const MAX_ENTRIES: usize = 64;
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_INPUT_BYTES: usize = 16 * 1024 - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubmissionState {
    Pending,
    Accepted,
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submission {
    pub id: u64,
    pub stream: String,
    pub text: String,
    pub state: SubmissionState,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    entries: VecDeque<Submission>,
    next_id: u64,
}

impl History {
    #[cfg(test)]
    pub fn entries(&self) -> impl Iterator<Item = &Submission> {
        self.entries.iter()
    }

    fn retained_bytes(&self) -> usize {
        MAX_ENTRIES * std::mem::size_of::<Submission>()
            + self
                .entries
                .iter()
                .map(|entry| entry.text.len() + entry.stream.len())
                .sum::<usize>()
    }

    fn push(&mut self, stream: String, text: String) -> Result<u64> {
        ensure!(
            !text.is_empty() && text.len() <= MAX_INPUT_BYTES,
            "一次输入最多 16 KiB，请缩短后发送。"
        );
        ensure!(
            stream.len() + text.len() + MAX_ENTRIES * std::mem::size_of::<Submission>()
                <= MAX_BYTES,
            "输入记录过大"
        );
        while self.entries.len() >= MAX_ENTRIES
            || self.retained_bytes() + stream.len() + text.len() > MAX_BYTES
        {
            self.entries.pop_front();
        }
        self.next_id = self.next_id.checked_add(1).context("输入记录编号已用尽")?;
        let id = self.next_id;
        // Copy into exact-sized strings rather than retaining oversized caller buffers.
        self.entries.push_back(Submission {
            id,
            stream: stream.as_str().into(),
            text: text.as_str().into(),
            state: SubmissionState::Pending,
        });
        Ok(id)
    }

    fn settle(&mut self, id: u64, state: SubmissionState) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id)
            && entry.state == SubmissionState::Pending
        {
            entry.state = state;
            return true;
        }
        false
    }

    pub(super) fn interrupted(&mut self) {
        for entry in &mut self.entries {
            if entry.state == SubmissionState::Pending {
                entry.state = SubmissionState::Unknown;
            }
        }
    }

    pub(super) fn remove_stream(&mut self, stream: &str) {
        self.entries.retain(|entry| entry.stream != stream);
    }

    pub(super) fn restore_stream(&mut self, previous: &str, stream: &str) {
        for entry in &mut self.entries {
            if entry.stream == previous {
                entry.stream = stream.into();
            }
        }
        while self.retained_bytes() > MAX_BYTES {
            self.entries.pop_front();
        }
    }
}

impl Actor {
    pub(super) async fn submit_input(&mut self, stream: String, text: String) -> Result<()> {
        let id = self.model.interaction.push(stream.clone(), text.clone())?;
        let operation = Operation::TerminalInput {
            stream,
            data: protocol::encode(format!("{text}\r").as_bytes()),
        };
        if let Err(error) = self.validate(&operation) {
            self.model.interaction.settle(id, SubmissionState::Rejected);
            return Err(error);
        }
        // These failures occur before any transport write and have a known outcome.
        if self.session.is_none() || self.outstanding.len() >= 64 {
            self.model.interaction.settle(id, SubmissionState::Rejected);
            bail!("输入未发送：连接不可用或待确认请求过多");
        }
        self.publish().await;
        self.request(operation, Pending::Input(id)).await
    }

    pub(super) fn input_response(&mut self, pending: &Option<Pending>, event: &Event) -> bool {
        let Some(Pending::Input(id)) = pending else {
            return false;
        };
        let state = match event {
            Event::Ok => SubmissionState::Accepted,
            Event::Error {
                outcome_unknown: true,
                ..
            } => SubmissionState::Unknown,
            Event::Error { .. } => SubmissionState::Rejected,
            _ => return false,
        };
        self.model.interaction.settle(*id, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor() -> Actor {
        Actor::new(mpsc::channel(32).0)
    }

    #[test]
    fn retained_history_is_bounded_by_bytes_and_count_and_is_stream_scoped() {
        let mut history = History::default();
        for index in 0..200 {
            history.push("pane".into(), format!("{index}")).unwrap();
        }
        assert_eq!(history.entries().count(), MAX_ENTRIES);
        assert_eq!(history.entries().next().unwrap().text, "136");
        for _ in 0..100 {
            history
                .push("pane".into(), "猫".repeat(MAX_INPUT_BYTES / 3))
                .unwrap();
            assert!(history.retained_bytes() <= MAX_BYTES);
            assert!(history.entries().count() <= MAX_ENTRIES);
        }
        assert!(
            history
                .push("pane".into(), "x".repeat(MAX_INPUT_BYTES + 1))
                .is_err()
        );
        let kept = history.push("other".into(), "kept".into()).unwrap();
        history.remove_stream("pane");
        assert_eq!(history.entries().count(), 1);
        assert_eq!(history.entries().next().unwrap().id, kept);
    }

    #[tokio::test]
    async fn only_matching_ok_accepts_input_and_disconnect_makes_pending_unknown() {
        let mut actor = actor();
        let id = actor
            .model
            .interaction
            .push("pane".into(), "printf safe".into())
            .unwrap();
        assert_eq!(
            actor.model.interaction.entries().next().unwrap().state,
            SubmissionState::Pending
        );
        actor
            .response(Response {
                id: 8,
                event: Event::Ok,
            })
            .await
            .unwrap();
        assert_eq!(
            actor.model.interaction.entries().next().unwrap().state,
            SubmissionState::Pending
        );
        actor.outstanding.insert(
            9,
            Outstanding {
                kind: Pending::Input(id),
                expires: Instant::now(),
            },
        );
        assert!(
            actor
                .response(Response {
                    id: 9,
                    event: Event::Ok
                })
                .await
                .unwrap()
        );
        assert_eq!(
            actor.model.interaction.entries().next().unwrap().state,
            SubmissionState::Accepted
        );
        let unknown = actor
            .model
            .interaction
            .push("pane".into(), "do not replay".into())
            .unwrap();
        actor.outstanding.insert(
            10,
            Outstanding {
                kind: Pending::Input(unknown),
                expires: Instant::now(),
            },
        );
        actor.disconnected().await;
        assert!(actor.outstanding.is_empty());
        let records = actor.model.interaction.entries().collect::<Vec<_>>();
        assert_eq!(records[0].state, SubmissionState::Accepted);
        assert_eq!(records[1].state, SubmissionState::Unknown);
        // A late acknowledgment from the disconnected channel cannot become acceptance.
        actor
            .response(Response {
                id: 10,
                event: Event::Ok,
            })
            .await
            .unwrap();
        assert_eq!(
            actor.model.interaction.entries().last().unwrap().state,
            SubmissionState::Unknown
        );
        assert!(actor.restoration_requests().is_empty());
    }

    #[tokio::test]
    async fn encrypted_request_has_exact_composer_text_and_no_queue_success_or_replay() {
        let (updates, mut receiver) = mpsc::channel(16);
        let mut actor = Actor::new(updates);
        actor.model.phase = Phase::Ready;
        actor.model.permission = Some(Permission::Operate);
        actor.model.panes.push(Pane {
            stream: "pane".into(),
            name: "shell".into(),
            kind: PaneKind::Terminal,
            herdr_grant: None,
            herdr_pane_id: None,
            programs: Vec::new(),
            controller: true,
            read_only: false,
            exit: None,
            unavailable: None,
            gap: false,
        });
        let (session, mut requests) = Session::observed();
        actor.session = Some(session.into());
        actor
            .command(Command::SubmitInput {
                stream: "pane".into(),
                text: "你好 Neko".into(),
            })
            .await
            .unwrap();
        let request = requests.recv().await.unwrap();
        let Operation::TerminalInput { stream, data } = request.operation else {
            panic!("not PTY input")
        };
        assert_eq!(stream, "pane");
        assert_eq!(
            protocol::decode(&data, 16 * 1024).unwrap(),
            "你好 Neko\r".as_bytes()
        );
        let Update::Model(model) = receiver.recv().await.unwrap() else {
            panic!("missing pending snapshot")
        };
        assert_eq!(
            model.interaction.entries().next().unwrap().state,
            SubmissionState::Pending
        );
        actor
            .response(Response {
                id: request.id,
                event: Event::Ok,
            })
            .await
            .unwrap();
        assert_eq!(
            actor.model.interaction.entries().next().unwrap().state,
            SubmissionState::Accepted
        );
        actor.disconnected().await;
        assert!(requests.try_recv().is_err());
        assert!(actor.outstanding.is_empty());
    }

    #[tokio::test]
    async fn host_errors_keep_known_rejection_distinct_from_unknown_outcome() {
        for (unknown, state) in [
            (false, SubmissionState::Rejected),
            (true, SubmissionState::Unknown),
        ] {
            let mut actor = actor();
            actor.model.phase = Phase::Ready;
            let id = actor
                .model
                .interaction
                .push("pane".into(), "test".into())
                .unwrap();
            actor.outstanding.insert(
                1,
                Outstanding {
                    kind: Pending::Input(id),
                    expires: Instant::now(),
                },
            );
            actor
                .response(Response {
                    id: 1,
                    event: Event::Error {
                        code: "REQUEST_FAILED".into(),
                        message: "fixture refusal".into(),
                        outcome_unknown: unknown,
                    },
                })
                .await
                .unwrap();
            assert_eq!(
                actor.model.interaction.entries().next().unwrap().state,
                state
            );
        }
    }

    #[tokio::test]
    async fn conversation_submission_cannot_bypass_control_or_readonly() {
        let mut actor = actor();
        actor.model.phase = Phase::Ready;
        actor.model.permission = Some(Permission::Operate);
        actor.model.panes.push(Pane {
            stream: "pane".into(),
            name: "shell".into(),
            kind: PaneKind::Terminal,
            herdr_grant: None,
            herdr_pane_id: None,
            programs: Vec::new(),
            controller: false,
            read_only: false,
            exit: None,
            unavailable: None,
            gap: false,
        });
        for (controller, read_only, permission, exited) in [
            (false, false, Permission::Operate, false),
            (true, true, Permission::Operate, false),
            (true, false, Permission::ReadOnly, false),
            (true, false, Permission::Operate, true),
        ] {
            actor.model.permission = Some(permission);
            let pane = &mut actor.model.panes[0];
            pane.controller = controller;
            pane.read_only = read_only;
            pane.exit = exited.then_some(0);
            assert!(
                actor
                    .submit_input("pane".into(), "not sent".into())
                    .await
                    .is_err()
            );
            assert_eq!(
                actor.model.interaction.entries().last().unwrap().state,
                SubmissionState::Rejected
            );
            assert!(actor.outstanding.is_empty());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn composer_text_reaches_real_local_pty_and_acknowledgment_is_separate_from_output() {
        // Shell startup is isolated in a child; never change this test process's HOME/cwd.
        const CHILD: &str = "NEKO_INTERACTION_PTY_CHILD";
        if std::env::var_os(CHILD).is_none() {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().unwrap();
            let shell = root.path().join("shell");
            std::fs::write(&shell, "#!/bin/sh\nexec /bin/sh\n").unwrap();
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "controller::interaction::tests::composer_text_reaches_real_local_pty_and_acknowledgment_is_separate_from_output", "--nocapture"])
                .env(CHILD, "1").env("SHELL", shell).env("HOME", root.path())
                .env_remove("ENV").env_remove("BASH_ENV").current_dir(root.path()).output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            tokio::time::timeout(Duration::from_secs(15), async {
                let host =
                    neoko_terminal_host::start(None, Arc::new(|_| Box::pin(async { Ok(()) })))
                        .await
                        .unwrap();
                let grant = host
                    .create_terminal("interaction test".into())
                    .await
                    .unwrap();
                let (updates, mut receiver) = mpsc::channel(128);
                let mut actor = Actor::new(updates);
                actor.model.phase = Phase::Ready;
                actor.model.local = true;
                actor.model.permission = Some(Permission::Operate);
                actor.model.resources = host.snapshot().resources;
                actor.session = Some(WorkspaceChannel::Local(host.local().await.unwrap()));
                actor
                    .request(
                        Operation::TerminalAttach {
                            grant: grant.clone(),
                            pane: None,
                        },
                        Pending::Attach(Attachment {
                            grant: grant.clone(),
                            pane: None,
                        }),
                    )
                    .await
                    .unwrap();
                loop {
                    let response = actor.session.as_mut().unwrap().receive().await.unwrap();
                    actor.response(response).await.unwrap();
                    if !actor.model.panes.is_empty() {
                        break;
                    }
                }
                let stream = actor.model.panes[0].stream.clone();
                actor
                    .request(
                        Operation::TerminalControl {
                            stream: stream.clone(),
                            take: true,
                        },
                        Pending::Ordinary,
                    )
                    .await
                    .unwrap();
                while !actor.model.panes[0].controller {
                    let response = actor.session.as_mut().unwrap().receive().await.unwrap();
                    actor.response(response).await.unwrap();
                }
                actor
                    .submit_input(stream, "printf '%s%s\\n' native_interaction_ ok".into())
                    .await
                    .unwrap();
                assert_eq!(
                    actor.model.interaction.entries().next().unwrap().state,
                    SubmissionState::Pending
                );
                let mut output = String::new();
                loop {
                    let response = actor.session.as_mut().unwrap().receive().await.unwrap();
                    actor.response(response).await.unwrap();
                    while let Ok(update) = receiver.try_recv() {
                        if let Update::Output { text, .. } = update {
                            output.push_str(&text);
                        }
                    }
                    if output.contains("native_interaction_ok")
                        && actor.model.interaction.entries().next().unwrap().state
                            == SubmissionState::Accepted
                    {
                        break;
                    }
                }
                host.close_terminal(&grant).await.unwrap();
                actor.disconnected().await;
            })
            .await
            .expect("real PTY input/ack/output timed out");
        });
    }
}
