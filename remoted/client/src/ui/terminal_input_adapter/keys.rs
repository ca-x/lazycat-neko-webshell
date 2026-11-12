//! xterm hardware keys. The native on_submit separately owns IME action keys.
use nsh_base::TerminalModes;

#[derive(Clone, Copy)]
pub(super) enum Bytes {
    Fixed(&'static str),
    Cursor(char),
}
impl Bytes {
    pub fn encode(self, modes: TerminalModes) -> String {
        match self {
            Self::Fixed(bytes) => bytes.to_owned(),
            Self::Cursor(code) if modes.application_cursor => format!("\x1bO{code}"),
            Self::Cursor(code) => format!("\x1b[{code}"),
        }
    }
}
pub(super) fn bindings(apple: bool) -> Vec<(&'static str, Bytes)> {
    use Bytes::*;
    let mut bindings = vec![
        ("enter", Fixed("\r")),
        ("backspace", Fixed("\x7f")),
        ("tab", Fixed("\t")),
        ("shift+tab", Fixed("\x1b[Z")),
        ("escape", Fixed("\x1b")),
        ("up", Cursor('A')),
        ("down", Cursor('B')),
        ("right", Cursor('C')),
        ("left", Cursor('D')),
        ("home", Cursor('H')),
        ("end", Cursor('F')),
        ("delete", Fixed("\x1b[3~")),
        ("pageup", Fixed("\x1b[5~")),
        ("pagedown", Fixed("\x1b[6~")),
        ("alt+left", Fixed("\x1bb")),
        ("alt+right", Fixed("\x1bf")),
        ("alt+backspace", Fixed("\x1b\x7f")),
    ];
    let controls = [
        ("ctrl+a", "\x01"),
        ("ctrl+b", "\x02"),
        ("ctrl+c", "\x03"),
        ("ctrl+d", "\x04"),
        ("ctrl+e", "\x05"),
        ("ctrl+f", "\x06"),
        ("ctrl+g", "\x07"),
        ("ctrl+h", "\x08"),
        ("ctrl+i", "\x09"),
        ("ctrl+j", "\x0a"),
        ("ctrl+k", "\x0b"),
        ("ctrl+l", "\x0c"),
        ("ctrl+m", "\x0d"),
        ("ctrl+n", "\x0e"),
        ("ctrl+o", "\x0f"),
        ("ctrl+p", "\x10"),
        ("ctrl+q", "\x11"),
        ("ctrl+r", "\x12"),
        ("ctrl+s", "\x13"),
        ("ctrl+t", "\x14"),
        ("ctrl+u", "\x15"),
        ("ctrl+v", "\x16"),
        ("ctrl+w", "\x17"),
        ("ctrl+x", "\x18"),
        ("ctrl+y", "\x19"),
        ("ctrl+z", "\x1a"),
    ];
    bindings.extend(
        controls
            .into_iter()
            .filter(|(key, _)| apple || *key != "ctrl+q")
            .map(|(key, data)| (key, Fixed(data))),
    );
    bindings
}
