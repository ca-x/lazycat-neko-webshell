use crate::portable;
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[test]
fn portable_reports_match_upstream_validation_and_metadata() {
    assert_eq!(
        portable::OscState::parse("blocked"),
        Some(portable::OscState::Blocked)
    );
    assert_eq!(
        portable::BlockedKind::parse("auth"),
        Some(portable::BlockedKind::Auth)
    );
    let mut payloads = vec![
        "?".to_owned(),
        "?:state=working".to_owned(),
        String::new(),
        "state=unknown".to_owned(),
        "state=working:progress=101".to_owned(),
        "state=blocked:kind=unknown".to_owned(),
        "state=idle:state=done".to_owned(),
        "state=working:id=agent//sub".to_owned(),
        "state=working:id=/agent".to_owned(),
        "state=working:app=not allowed".to_owned(),
        "state=working:title=???".to_owned(),
        format!("state=working:id={}", "x".repeat(129)),
        format!("state=working:app={}", "x".repeat(33)),
        format!("state=working:title={}", STANDARD.encode("x".repeat(193))),
        format!("state=working:msg={}", STANDARD.encode("x".repeat(2049))),
        format!("state=working:unknown={}", "x".repeat(4096)),
    ];
    for state in ["idle", "working", "blocked", "done", "error", "clear"] {
        payloads.push(format!("state={state}:id=agent/sub:kind=permission:progress=42:app=codex:title=5rWL6K+V:msg=QXBwcm92ZQ=="));
    }
    for text in ["测试", "hello\nworld\t!", "left\u{202e}right", "🦀", ""] {
        payloads.push(format!(
            "state=working:title={}:msg={}",
            STANDARD.encode(text),
            STANDARD.encode(text)
        ));
    }
    for payload in payloads {
        assert_eq!(
            format!("{:?}", portable::osc7501::parse_report(&payload)),
            format!("{:?}", w1::osc7501::parse_report(&payload)),
            "payload: {payload:?}",
        );
    }
}

fn compare_stream(bytes: &[u8], split: usize) {
    let mut upstream = w1::stream::OutputStream::default();
    let mut portable = portable::stream::OutputStream::default();
    let mut upstream_output = Vec::new();
    let mut portable_output = Vec::new();
    let mut upstream_captured = Vec::new();
    let mut portable_captured = Vec::new();
    for chunk in [&bytes[..split], &bytes[split..]] {
        upstream.process(chunk, &mut upstream_output, &mut upstream_captured);
        portable.process(chunk, &mut portable_output, &mut portable_captured);
        assert_eq!(
            portable_output, upstream_output,
            "split={split}, bytes={bytes:?}"
        );
        assert_eq!(
            format!("{portable_captured:?}"),
            format!("{upstream_captured:?}")
        );
        assert_eq!(portable.at_boundary(), upstream.at_boundary());
        assert_eq!(portable.take_reset(), upstream.take_reset());
    }
}

#[test]
fn portable_stream_matches_upstream_at_every_split_and_escape_boundary() {
    let inputs: &[&[u8]] = &[
        b"plain output",
        "UTF-8: 测试 🦀".as_bytes(),
        b"\x1b]7501;?\x07",
        b"\x1b]7501;?\x1b\\",
        b"\x1b]7501;?:capabilities\x07",
        b"before\x1b]7501;state=working:app=codex\x07after",
        b"\x1b]7501;state=blocked:kind=permission\x1b\\\x1b]7501;state=clear\x07",
        b"\x1b]7501;state=working\x18\x1b]7501;?\x07",
        b"\x1b]7501;state=working\x1a\x1b]7501;?\x1b\\",
        b"\x1b]7501;state=working\x1b[31mignored\x1b]7501;?\x07",
        b"\x1b]0;title\x07\x1b]7501;?\x07",
        b"\x1bPignored\x1b]7501;?\x07\x1b\\\x1b]7501;?\x07",
        b"\x1b_ignored\x1b]7501;?\x07\x1b\\\x1b]7501;state=done\x07",
        b"\x1b[31mcolor\x1b[0m\x1b]7501;?\x1b\\",
        b"\x1b]7501;state=working\x07\x1bc\x1b]7501;state=done\x07",
        b"\xc2\x9b\x1b]7501;?\x07\xff\x1b]7501;state=error\x07",
    ];
    for bytes in inputs {
        for split in 0..=bytes.len() {
            compare_stream(bytes, split);
        }
    }
    let mut oversized = b"\x1b]7501;state=working:".to_vec();
    oversized.extend_from_slice(&vec![b'x'; 8192]);
    oversized.extend_from_slice(b"\x07\x1b]7501;?\x1b\\");
    for split in [0, 1, 7, 4095, 4096, 4097, 8192, oversized.len()] {
        compare_stream(&oversized, split);
    }
}

#[test]
fn non_unix_detection_does_not_infer_or_claim_a_process_identity() {
    let mut automatic = crate::non_unix_automatic::Automatic;
    assert!(!automatic.foreground(Some(std::process::id()), 80, 24));
    automatic.output(b"codex Working...\x1b]7501;state=working\x07");
    automatic.input(b"\r\x03");
    automatic.resize(120, 40);
    assert!(automatic.record().is_none());
    assert!(!automatic.foreground(None, 120, 40));
    assert!(
        crate::non_unix_automatic::hook_identity("codex", std::process::id(), std::process::id())
            .is_none()
    );
}
