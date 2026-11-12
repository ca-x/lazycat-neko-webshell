use super::{
    CHUNK, IO_QUEUE, IoAction, IoCommand, REPLAY_BYTES, ReplyAuthority, Shared, State, Stop,
    failure_event, lock, publish_program_status, queue_event, queue_output, remove_subscriber,
    replies, writable,
};
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    sync::mpsc as blocking,
    time::{Duration, Instant},
};

pub(super) enum PtyOutput {
    Bytes(Vec<u8>),
    Done(Result<()>),
}

pub(super) fn accept_output(
    state: &mut State,
    bytes: &[u8],
    reply: impl FnOnce(Vec<u8>) -> Result<()>,
) -> Result<()> {
    let replies = state.program_status.observe(bytes);
    if !replies.is_empty() {
        reply(replies)?;
    }
    for &byte in bytes {
        if state.replay.len() == REPLAY_BYTES {
            state.replay.pop_front();
            state.truncated = true;
        }
        state.replay.push_back(byte);
    }
    let mut lost = Vec::new();
    for (stream, entry) in &mut state.attachments {
        if queue_output(entry, stream, bytes, false, false, false).is_err() {
            lost.push(stream.clone());
        }
    }
    for stream in lost {
        remove_subscriber(state, &stream, Stop::Gap);
    }
    publish_program_status(state);
    Ok(())
}

pub(super) fn read_output(
    reader: &mut Box<dyn Read + Send>,
    output: blocking::SyncSender<PtyOutput>,
) {
    let result = (|| -> Result<()> {
        let mut bytes = [0; CHUNK];
        loop {
            let count = match reader.read(&mut bytes) {
                Ok(0) => return Ok(()),
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                #[cfg(unix)]
                Err(error) if error.raw_os_error() == Some(5) => return Ok(()),
                Err(error) => return Err(error).context("PTY output failed"),
            };
            if output
                .send(PtyOutput::Bytes(bytes[..count].to_vec()))
                .is_err()
            {
                return Ok(());
            }
        }
    })();
    let _ = output.send(PtyOutput::Done(result));
}

fn output_done(shared: &Shared, result: Result<()>) {
    if let Ok(mut state) = shared.state.lock() {
        state.read_done = true;
        if let Err(failure) = result {
            let failure = format!("{failure:#}");
            state.failure = Some(failure.clone());
            let streams: Vec<_> = state.attachments.keys().cloned().collect();
            for stream in streams {
                queue_event(&mut state, &stream, failure_event(&failure));
            }
        }
        shared.drained.notify_all();
    }
}

pub(super) fn write_commands(
    mut writer: Box<dyn Write + Send>,
    commands: blocking::Receiver<IoCommand>,
    output: blocking::Receiver<PtyOutput>,
    replies: blocking::Receiver<Vec<u8>>,
    shared: &Shared,
    authority: &ReplyAuthority,
) {
    let result = (|| {
        let dimensions = lock(&shared.state)?.dimensions;
        replies::with(dimensions.0, dimensions.1, |vt| {
            run(
                vt,
                writer.as_mut(),
                commands,
                output,
                replies,
                shared,
                authority,
            )
        })
    })();
    output_done(shared, result);
}

fn write_replies(writer: &mut dyn Write, replies: &blocking::Receiver<Vec<u8>>) -> Result<()> {
    for reply in replies.try_iter().take(IO_QUEUE) {
        writer
            .write_all(&reply)
            .and_then(|()| writer.flush())
            .context("PTY support reply failed")?;
    }
    Ok(())
}

fn run(
    vt: &mut replies::Replies<'_>,
    writer: &mut dyn Write,
    commands: blocking::Receiver<IoCommand>,
    output: blocking::Receiver<PtyOutput>,
    replies: blocking::Receiver<Vec<u8>>,
    shared: &Shared,
    authority: &ReplyAuthority,
) -> Result<()> {
    let mut last_poll = Instant::now() - Duration::from_secs(1);
    loop {
        for _ in 0..IO_QUEUE {
            match output.try_recv() {
                Ok(PtyOutput::Bytes(bytes)) => {
                    {
                        let mut state = lock(&shared.state)?;
                        if state.finished {
                            continue;
                        }
                        authority.enqueue(vt.observe(&bytes)?)?;
                        authority.foreground(&mut state.program_status);
                        accept_output(&mut state, &bytes, |reply| authority.enqueue(reply))?;
                    }
                    write_replies(writer, &replies)?;
                }
                Ok(PtyOutput::Done(result)) => {
                    result?;
                    output_done(shared, Ok(()));
                }
                Err(blocking::TryRecvError::Empty) => break,
                Err(blocking::TryRecvError::Disconnected) => {
                    ensure!(lock(&shared.state)?.read_done, "PTY output worker stopped");
                    break;
                }
            }
        }
        if last_poll.elapsed() >= Duration::from_millis(500) {
            if let Ok(mut state) = shared.state.lock()
                && !state.finished
                && !state.read_done
            {
                authority.foreground(&mut state.program_status);
                publish_program_status(&mut state);
            }
            last_poll = Instant::now();
        }
        let command = match commands.recv_timeout(Duration::from_millis(25)) {
            Ok(command) => command,
            Err(blocking::RecvTimeoutError::Timeout) => continue,
            Err(blocking::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        if command.completed.is_closed() {
            continue;
        }
        let writable = writable(&*lock(&shared.state)?, &command.stream, &command.subscriber);
        if let Err(error) = writable {
            let _ = command.completed.send(Err(error));
            continue;
        }
        match command.action {
            IoAction::Input(data) => {
                {
                    let mut state = lock(&shared.state)?;
                    state.program_status.input(&data);
                    publish_program_status(&mut state);
                }
                let result = writer
                    .write_all(&data)
                    .and_then(|()| writer.flush())
                    .context("PTY write failed; outcome unknown, do not replay");
                let _ = command.completed.send(result);
            }
            IoAction::Resize(dimensions) => {
                let result = (|| -> Result<()> {
                    let mut state = lock(&shared.state)?;
                    lock(&authority.master)?
                        .resize(dimensions)
                        .context("PTY resize failed")?;
                    state.dimensions = (dimensions.cols, dimensions.rows);
                    state
                        .program_status
                        .resize(dimensions.cols, dimensions.rows);
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = command.completed.send(Err(error));
                    continue;
                }
                let result = vt
                    .resize(dimensions.cols, dimensions.rows)
                    .and_then(|reply| authority.enqueue(reply))
                    .and_then(|()| write_replies(writer, &replies));
                if let Err(error) = result {
                    let _ = command.completed.send(Err(anyhow::anyhow!("{error:#}")));
                    return Err(error);
                }
                let _ = command.completed.send(Ok(()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminals::{EVENT_QUEUE, Subscriber, size};
    use neko_remoted_protocol::messages::Event;
    use portable_pty::{PtySize, native_pty_system};
    use std::sync::{Arc, Mutex};
    use tokio::sync::{mpsc, oneshot, watch};

    fn fixture() -> (
        Shared,
        ReplyAuthority,
        blocking::Receiver<Vec<u8>>,
        mpsc::Receiver<Event>,
    ) {
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let (replies, reply_input) = blocking::sync_channel(IO_QUEUE);
        let authority = ReplyAuthority {
            master: Arc::new(Mutex::new(pair.master)),
            replies,
        };
        let (events, incoming) = mpsc::channel(EVENT_QUEUE);
        let (stop, _stopped) = watch::channel(Stop::Running);
        let mut state = State {
            dimensions: (80, 24),
            controller: Some("controller".into()),
            ..State::default()
        };
        state.attachments.insert(
            "controller".into(),
            Subscriber {
                owner: "owner".into(),
                read_only: false,
                events,
                stop,
                sequence: 0,
            },
        );
        let shared = Shared {
            state: Mutex::new(state),
            ..Shared::default()
        };
        (shared, authority, reply_input, incoming)
    }

    struct ReadFailure;

    impl Read for ReadFailure {
        fn read(&mut self, _bytes: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected read failure"))
        }
    }

    #[test]
    fn read_error_follows_queued_bytes_without_fabricating_exit() {
        let (shared, authority, replies, mut incoming) = fixture();
        let mut reader: Box<dyn Read + Send> =
            Box::new(std::io::Cursor::new(b"final queued bytes".to_vec()).chain(ReadFailure));
        let (output, chunks) = blocking::sync_channel(IO_QUEUE);
        read_output(&mut reader, output);
        let (_commands, commands) = blocking::sync_channel(IO_QUEUE);
        write_commands(
            Box::new(std::io::sink()),
            commands,
            chunks,
            replies,
            &shared,
            &authority,
        );
        let state = shared.state.lock().unwrap();
        assert!(state.read_done);
        assert_eq!(state.exit, None);
        assert_eq!(
            state.replay.iter().copied().collect::<Vec<_>>(),
            b"final queued bytes"
        );
        assert!(
            state
                .failure
                .as_ref()
                .unwrap()
                .contains("injected read failure")
        );
        assert!(matches!(
            incoming.try_recv(),
            Ok(Event::TerminalOutput { .. })
        ));
        assert!(
            matches!(incoming.try_recv(), Ok(Event::Error { code, .. }) if code == "TERMINAL_ERROR")
        );
        assert!(incoming.try_recv().is_err());
    }

    #[derive(Default)]
    struct FailResizeReport {
        written: bool,
    }

    impl Write for FailResizeReport {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.written {
                return Err(std::io::Error::other("injected resize report failure"));
            }
            self.written = true;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn resize_report_write_error_fails_the_request_and_authority() {
        let (shared, authority, replies, _incoming) = fixture();
        let (output, chunks) = blocking::sync_channel(IO_QUEUE);
        output
            .send(PtyOutput::Bytes(b"\x1b[?2048h".to_vec()))
            .unwrap();
        let (commands, requests) = blocking::sync_channel(IO_QUEUE);
        let (completed, result) = oneshot::channel();
        commands
            .send(IoCommand {
                stream: "controller".into(),
                subscriber: "owner".into(),
                action: IoAction::Resize(size(40, 12).unwrap()),
                completed,
            })
            .unwrap();
        write_commands(
            Box::<FailResizeReport>::default(),
            requests,
            chunks,
            replies,
            &shared,
            &authority,
        );
        let error = result.blocking_recv().unwrap().unwrap_err();
        assert!(format!("{error:#}").contains("injected resize report failure"));
        let state = shared.state.lock().unwrap();
        assert!(state.read_done);
        assert_eq!(state.exit, None);
        assert!(
            state
                .failure
                .as_ref()
                .unwrap()
                .contains("PTY support reply failed")
        );
    }
}
