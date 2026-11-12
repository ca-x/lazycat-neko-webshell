use super::*;

#[test]
fn ghostty_answers_fragmented_da_dsr_and_cursor_once() {
    with(80, 24, |replies| {
        assert!(replies.observe(b"\x1b[")?.is_empty());
        let da = replies.observe(b"c")?;
        assert!(da.starts_with(b"\x1b[?") && da.ends_with(b"c"), "{da:?}");
        assert!(replies.observe(b"")?.is_empty());
        assert_eq!(replies.observe(b"\x1b[5n")?, b"\x1b[0n");
        assert_eq!(replies.observe(b"\x1b[7;13H\x1b[6n")?, b"\x1b[7;13R");
        assert_eq!(replies.observe(b"\x1b[?6n")?, b"\x1b[?7;13;1R");
        // Ordinary bytes and the independent program-status protocol do not
        // generate duplicate standard replies through this authority.
        assert!(replies.observe(b"text\x1b]7501;?\x07")?.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn ghostty_resize_updates_reports_and_clamps_cursor() {
    with(80, 24, |replies| {
        assert!(replies.observe(b"\x1b[24;80H")?.is_empty());
        assert!(replies.resize(40, 12)?.is_empty());
        assert_eq!(replies.observe(b"\x1b[18t")?, b"\x1b[8;12;40t");
        let cursor = replies.observe(b"\x1b[6n")?;
        assert_eq!(cursor, b"\x1b[12;40R");
        let enabled = replies.observe(b"\x1b[?2048h")?;
        assert!(
            !enabled.is_empty(),
            "Ghostty must emit initial in-band size report"
        );
        let resized = replies.resize(100, 30)?;
        assert!(
            !resized.is_empty(),
            "Ghostty must emit enabled in-band resize report"
        );
        assert!(replies.observe(b"")?.is_empty());
        assert!(
            replies.resize(100, 30)?.is_empty(),
            "unchanged size must not send a duplicate report"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn reply_flood_fails_atomically_and_stays_failed() {
    with(80, 24, |replies| {
        let flood = b"\x1b[c".repeat(INPUT_BYTES / 3);
        let error = replies.observe(&flood).unwrap_err();
        assert!(error.to_string().contains("reply byte budget"), "{error:#}");
        assert!(
            replies
                .observe(b"\x1b[5n")
                .unwrap_err()
                .to_string()
                .contains("previously failed")
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn screen_and_input_limits_fail_explicitly() {
    assert!(with(0, 24, |_| Ok(())).is_err());
    assert!(with(1024, 1024, |_| Ok(())).is_err());
    with(80, 24, |replies| {
        assert!(replies.resize(1024, 1024).is_err());
        assert!(replies.observe(b"\x1b[c").is_err());
        Ok(())
    })
    .unwrap();
    with(80, 24, |replies| {
        assert!(replies.observe(&vec![b'x'; INPUT_BYTES + 1]).is_err());
        assert!(replies.resize(80, 24).is_err());
        Ok(())
    })
    .unwrap();
}

#[test]
fn dimensions_enforce_the_public_axis_and_screen_limits() {
    assert!(dimensions(512, 256).is_ok());
    assert!(dimensions(1000, 131).is_ok());
    assert!(dimensions(512, 257).is_err());
    assert!(dimensions(1000, 132).is_err());
    assert!(dimensions(1001, 1).is_err());
    assert!(dimensions(1, 1001).is_err());
    assert!(dimensions(1, 0).is_err());
}

#[test]
fn allocator_refuses_excess_and_reclaims_live_memory() {
    let budget = Budget {
        limit: 64,
        used: Cell::new(0),
        exhausted: Cell::new(false),
    };
    let layout = Layout::from_size_align(64, 8).unwrap();
    let allocation = budget.allocate(layout).unwrap();
    assert_eq!(budget.used.get(), 64);
    assert!(budget.allocate(Layout::new::<u8>()).is_err());
    assert!(budget.exhausted.get());
    // SAFETY: This is precisely the pointer/layout allocated above.
    unsafe { budget.deallocate(allocation.cast(), layout) };
    assert_eq!(budget.used.get(), 0);
    assert!(with_budget(80, 24, 1, |_| Ok(())).is_err());
}

#[test]
fn ghostty_allocation_failure_during_feed_is_explicit_and_sticky() {
    with(80, 24, |replies| {
        let remaining = replies.budget.limit - replies.budget.used.get();
        let layout = Layout::from_size_align(remaining, 8).unwrap();
        let allocation = replies.budget.allocate(layout).unwrap();
        // Entering a fresh alternate screen forces Ghostty to request memory.
        let result = replies.observe(b"\x1b[?1049h");
        // SAFETY: Free the exact test-owned allocation before checking results.
        unsafe { replies.budget.deallocate(allocation.cast(), layout) };
        let error = result.unwrap_err();
        assert!(
            error.to_string().contains("memory budget exceeded"),
            "{error:#}"
        );
        assert!(
            replies
                .observe(b"\x1b[c")
                .unwrap_err()
                .to_string()
                .contains("previously failed")
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn scrollback_and_retained_sequences_remain_bounded() {
    with(80, 24, |replies| {
        for _ in 0..100 {
            assert!(replies.observe(&b"line\r\n".repeat(1000))?.is_empty());
        }
        assert!(replies.terminal.scrollback_rows()? < 200);
        assert!(replies.budget.used.get() < MEMORY_BYTES);
        // An unfinished control string cannot grow without limit across reads.
        replies.observe(b"\x1b]0;")?;
        for _ in 0..100 {
            replies.observe(&[b'x'; INPUT_BYTES])?;
        }
        replies.observe(b"\x07")?;
        assert!(replies.budget.used.get() < MEMORY_BYTES);
        Ok(())
    })
    .unwrap();
}

#[cfg(unix)]
#[test]
fn real_pty_probe_receives_one_ghostty_reply_without_renderer() {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::{
        io::{Read, Write},
        sync::mpsc,
        thread,
        time::Duration,
    };
    let expected = with(80, 24, |replies| replies.observe(b"\x1b[c")).unwrap();
    let pair = native_pty_system().openpty(PtySize::default()).unwrap();
    let mut command = CommandBuilder::new("/bin/sh");
    command.arg("-c");
    command.arg(format!("stty -echo -icanon min 0 time 10; printf '\\033[c'; dd bs=1 count={} 2>/dev/null | od -An -tx1; printf 'PROBE_DONE\\n'", expected.len()));
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (sent, received) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let result = with(80, 24, |replies| {
            let mut output = Vec::new();
            let mut reply_count = 0;
            let mut bytes = [0; 1024];
            loop {
                let n = match reader.read(&mut bytes) {
                    Ok(n) => n,
                    Err(e) if e.raw_os_error() == Some(5) => 0,
                    Err(e) => return Err(e.into()),
                };
                if n == 0 {
                    break;
                }
                ensure!(output.len() + n <= 64 * 1024, "test PTY output overflow");
                let reply = replies.observe(&bytes[..n])?;
                if !reply.is_empty() {
                    writer.write_all(&reply)?;
                    writer.flush()?;
                    reply_count += 1;
                }
                output.extend_from_slice(&bytes[..n]);
                if output.windows(10).any(|w| w == b"PROBE_DONE") {
                    break;
                }
            }
            Ok((output, reply_count))
        });
        let _ = sent.send(result);
    });
    let result = received.recv_timeout(Duration::from_secs(5));
    if result.is_err() {
        let _ = child.kill();
    }
    drop(pair.master);
    let status = child.wait().unwrap();
    worker.join().unwrap();
    let (output, count) = result.unwrap().unwrap();
    assert!(status.success());
    assert_eq!(
        count, 1,
        "replay/subscriber delivery must not invoke observe again"
    );
    let hex = expected
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&output);
    let received_hex: Vec<_> = text
        .split_whitespace()
        .filter(|token| token.len() == 2 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    assert_eq!(
        received_hex, hex,
        "PTY received different reply bytes: {text}"
    );
    assert!(text.contains("PROBE_DONE"), "{text}");
}
