//! Native key frames use Herdr's raw `send_text`; pasted text uses `send_input`.
use serde_json::{Value, json};

pub(super) fn request(pane: &str, data: &[u8]) -> Result<(&'static str, Value), String> {
    const BEGIN: &[u8] = b"\x1b[200~";
    const END: &[u8] = b"\x1b[201~";
    if let Some(body) = data.strip_prefix(BEGIN) {
        let body = body
            .strip_suffix(END)
            .ok_or("incomplete Herdr bracketed paste")?;
        return Ok((
            "pane.send_input",
            json!({"pane_id":pane,"text":paste_text(body)?}),
        ));
    }
    if known_key(data) {
        // send_input.text is a paste and adds wrappers in bracketed-paste mode.
        // send_text writes the original bytes, including application-cursor forms.
        let text = std::str::from_utf8(data).map_err(|_| "Herdr key input must be valid UTF-8")?;
        return Ok(("pane.send_text", json!({"pane_id":pane,"text":text})));
    }
    let text = paste_text(data)?;
    Ok(("pane.send_text", json!({"pane_id":pane,"text":text})))
}

fn paste_text(data: &[u8]) -> Result<&str, String> {
    let text = std::str::from_utf8(data).map_err(|_| "Herdr text input must be valid UTF-8")?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("unknown or embedded Herdr terminal control sequence".into());
    }
    Ok(text)
}

fn known_key(data: &[u8]) -> bool {
    matches!(
        data,
        b"\r"
            | b"\r\n"
            | b"\t"
            | b"\x7f"
            | b"\x1b"
            | b"\x1b[A"
            | b"\x1bOA"
            | b"\x1b[B"
            | b"\x1bOB"
            | b"\x1b[C"
            | b"\x1bOC"
            | b"\x1b[D"
            | b"\x1bOD"
            | b"\x1b[H"
            | b"\x1bOH"
            | b"\x1b[1~"
            | b"\x1b[7~"
            | b"\x1b[F"
            | b"\x1bOF"
            | b"\x1b[4~"
            | b"\x1b[8~"
            | b"\x1b[2~"
            | b"\x1b[3~"
            | b"\x1b[5~"
            | b"\x1b[6~"
            | b"\x1b[Z"
            | b"\x1bb"
            | b"\x1bf"
            | b"\x1b\x7f"
            | b"\x1bOP"
            | b"\x1b[11~"
            | b"\x1bOQ"
            | b"\x1b[12~"
            | b"\x1bOR"
            | b"\x1b[13~"
            | b"\x1bOS"
            | b"\x1b[14~"
            | b"\x1b[15~"
            | b"\x1b[17~"
            | b"\x1b[18~"
            | b"\x1b[19~"
            | b"\x1b[20~"
            | b"\x1b[21~"
            | b"\x1b[23~"
            | b"\x1b[24~"
            | [1..=26]
    )
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde_json::json;

    fn convert(bytes: &[u8]) -> Result<(&'static str, serde_json::Value), String> {
        super::super::terminal_mcp_input_request("w1:p1", &STANDARD.encode(bytes))
    }

    #[test]
    fn preserves_nsh_normal_application_navigation_alt_and_function_key_bytes() {
        let cases = [
            "\x1b[A", "\x1bOA", "\x1b[B", "\x1bOB", "\x1b[C", "\x1bOC", "\x1b[D", "\x1bOD",
            "\x1b[H", "\x1bOH", "\x1b[F", "\x1bOF", "\x1b[1~", "\x1b[4~", "\x1b[2~", "\x1b[3~",
            "\x1b[5~", "\x1b[6~", "\x1b[Z", "\x1bb", "\x1bf", "\x1b\x7f", "\x1bOP", "\x1bOQ",
            "\x1bOR", "\x1bOS", "\x1b[15~", "\x1b[17~", "\x1b[18~", "\x1b[19~", "\x1b[20~",
            "\x1b[21~", "\x1b[23~", "\x1b[24~",
        ];
        for bytes in cases {
            let (method, params) = convert(bytes.as_bytes()).unwrap();
            assert_eq!(method, "pane.send_text");
            assert_eq!(params, json!({"pane_id":"w1:p1","text":bytes}), "{bytes:?}");
        }
        for byte in 1..=26 {
            let (method, params) = convert(&[byte]).unwrap();
            assert_eq!(method, "pane.send_text");
            assert_eq!(params["text"].as_str().unwrap().as_bytes(), &[byte]);
        }
    }

    #[test]
    fn bracketed_paste_is_exact_text_and_never_submits_a_trailing_newline() {
        for body in ["你好 👋\n", "\t  spaces\r\n", "", "line one\nline two\n"] {
            let bytes = format!("\x1b[200~{body}\x1b[201~");
            assert_eq!(
                convert(bytes.as_bytes()).unwrap(),
                ("pane.send_input", json!({"pane_id":"w1:p1","text":body}))
            );
        }
        assert_eq!(
            convert("普通 UTF8 👋".as_bytes()).unwrap().1["text"],
            "普通 UTF8 👋"
        );
    }

    #[test]
    fn unwrapped_typing_and_newlines_are_raw_without_paste_or_synthetic_enter() {
        for text in [
            "j",
            "普通 UTF8 👋",
            "printf hello\n",
            "line one\nline two\n",
        ] {
            assert_eq!(
                convert(text.as_bytes()).unwrap(),
                ("pane.send_text", json!({"pane_id":"w1:p1", "text":text}))
            );
        }
    }

    #[test]
    fn unknown_malformed_and_embedded_controls_are_rejected_before_paste() {
        for bytes in [
            b"\x1b[999~".as_slice(),
            b"\x1b[",
            b"text\x1b[A",
            b"\x1b[200~text",
            b"\x1b[201~",
            b"\x1b[200~text\x1b[A\x1b[201~",
            b"text\x03",
            b"\0",
        ] {
            assert!(
                convert(bytes).is_err(),
                "{bytes:?} must not become pasted text"
            );
        }
    }
}
