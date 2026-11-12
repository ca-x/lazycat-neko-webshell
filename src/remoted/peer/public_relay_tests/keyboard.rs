//! Captures actual bytes delivered by Herdr to a raw-mode PTY through public Relay.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

fn cases() -> Vec<(&'static [u8], &'static [u8])> {
    vec![
        (b"\x1b[A", b"\x1b[A"),
        (b"\x1bOA", b"\x1bOA"),
        (b"\x1b[B", b"\x1b[B"),
        (b"\x1bOB", b"\x1bOB"),
        (b"\x1b[C", b"\x1b[C"),
        (b"\x1bOC", b"\x1bOC"),
        (b"\x1b[D", b"\x1b[D"),
        (b"\x1bOD", b"\x1bOD"),
        (b"\x1b[H", b"\x1b[H"),
        (b"\x1bOH", b"\x1bOH"),
        (b"\x1b[F", b"\x1b[F"),
        (b"\x1bOF", b"\x1bOF"),
        (b"\x1b[2~", b"\x1b[2~"),
        (b"\x1b[3~", b"\x1b[3~"),
        (b"\x1b[5~", b"\x1b[5~"),
        (b"\x1b[6~", b"\x1b[6~"),
        (b"\x1b[Z", b"\x1b[Z"),
        (b"\x1bb", b"\x1bb"),
        (b"\x1bf", b"\x1bf"),
        (b"\x1b\x7f", b"\x1b\x7f"),
        (b"\x1bOP", b"\x1bOP"),
        (b"\x1bOQ", b"\x1bOQ"),
        (b"\x1bOR", b"\x1bOR"),
        (b"\x1bOS", b"\x1bOS"),
        (b"\x1b[15~", b"\x1b[15~"),
        (b"\x1b[17~", b"\x1b[17~"),
        (b"\x1b[18~", b"\x1b[18~"),
        (b"\x1b[19~", b"\x1b[19~"),
        (b"\x1b[20~", b"\x1b[20~"),
        (b"\x1b[21~", b"\x1b[21~"),
        (b"\x1b[23~", b"\x1b[23~"),
        (b"\x1b[24~", b"\x1b[24~"),
        (b"\x01", b"\x01"),
        (b"\x02", b"\x02"),
        (b"\x03", b"\x03"),
        (b"\x04", b"\x04"),
        (b"\x05", b"\x05"),
        (b"\x06", b"\x06"),
        (b"\x07", b"\x07"),
        (b"\x08", b"\x08"),
        (b"\x09", b"\x09"),
        (b"\x0a", b"\x0a"),
        (b"\x0b", b"\x0b"),
        (b"\x0c", b"\x0c"),
        (b"\x0d", b"\x0d"),
        (b"\x0e", b"\x0e"),
        (b"\x0f", b"\x0f"),
        (b"\x10", b"\x10"),
        (b"\x11", b"\x11"),
        (b"\x12", b"\x12"),
        (b"\x13", b"\x13"),
        (b"\x14", b"\x14"),
        (b"\x15", b"\x15"),
        (b"\x16", b"\x16"),
        (b"\x17", b"\x17"),
        (b"\x18", b"\x18"),
        (b"\x19", b"\x19"),
        (b"\x1a", b"\x1a"),
        (b"\r", b"\r"),
        (b"\t", b"\t"),
        (b"\x7f", b"\x7f"),
        (b"\x1b", b"\x1b"),
        ("普通 UTF8 👋".as_bytes(), "普通 UTF8 👋".as_bytes()),
        (
            "\x1b[200~中文粘贴\n末尾换行\n\x1b[201~".as_bytes(),
            "\x1b[200~中文粘贴\n末尾换行\n\x1b[201~".as_bytes(),
        ),
    ]
}

async fn input(client: &mut Client, stream: &str, bytes: &[u8]) -> Event {
    let id = client
        .request(Operation::TerminalInput {
            stream: stream.into(),
            data: protocol::encode(bytes),
        })
        .await;
    client.response(id).await
}

pub(super) async fn run(client: &mut Client, stream: &str) {
    let cases = cases();
    let expected: Vec<u8> = cases
        .iter()
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect();
    let script = format!(
        r"import os,sys,tty,termios,select,time,base64,json
fd=sys.stdin.fileno()
old=termios.tcgetattr(fd)
expected=base64.b64decode({:?})
data=b''
try:
 tty.setraw(fd)
 os.write(1,b'\x1b[?1h\x1b[?2004hKEY-CAPTURE-READY\r\n')
 deadline=time.monotonic()+100
 while len(data)<len(expected) and time.monotonic()<deadline:
  if select.select([fd],[],[],1)[0]: data+=os.read(fd,16384)
 if select.select([fd],[],[],0.5)[0]: data+=os.read(fd,16384)
 json.dump({{'expected_hex':expected.hex(),'actual_hex':data.hex(),'passed':data==expected}},open('native-key-capture.json','w'))
finally:
 termios.tcsetattr(fd,termios.TCSANOW,old)
 os.write(1,b'\x1b[?1l\x1b[?2004l')
print('KEY-CAPTURE-PASS' if data==expected else 'KEY-CAPTURE-FAIL:'+data.hex(),flush=True)
",
        STANDARD.encode(&expected)
    );
    let command = format!("python3 -u -c '{}'\n", script.replace('\'', "'\\''"));
    assert!(matches!(
        input(client, stream, command.as_bytes()).await,
        Event::Ok
    ));
    client.output(stream, b"KEY-CAPTURE-READY").await;
    assert!(matches!(
        input(client, stream, b"\x1b[999~").await,
        Event::Error { .. }
    ));
    for (bytes, _) in &cases {
        let event = input(client, stream, bytes).await;
        assert!(matches!(event, Event::Ok), "key frame {bytes:?}: {event:?}");
    }
    client.output(stream, b"KEY-CAPTURE-PASS").await;
    eprintln!(
        "PASS public encrypted genuine Herdr raw PTY captured {} navigation/function/Alt/application/UTF8/paste frames byte-for-byte; unknown escape rejected; bracketed paste trailing newline did not submit Enter",
        cases.len()
    );
}
