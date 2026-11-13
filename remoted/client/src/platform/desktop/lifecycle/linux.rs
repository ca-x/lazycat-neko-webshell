//! Bounded checks of the current process's registered StatusNotifierItem.

use serde_json::Value;
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_PATH: &str = "/StatusNotifierItem";

pub(super) async fn available() -> bool {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if std::thread::Builder::new()
        .name("neoko-tray-probe".into())
        .spawn(move || {
            let _ = sender.send(probe(std::process::id(), busctl));
        })
        .is_err()
    {
        return false;
    }
    receiver.await.unwrap_or(false)
}

fn probe(pid: u32, mut call: impl FnMut(&[&str]) -> Option<Value>) -> bool {
    let Some(names) = call(&[
        "call",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
    ]) else {
        return false;
    };
    // Read this process's concrete NSH bus name rather than another app's tray.
    let prefix = format!("org.kde.StatusNotifierItem-{pid}-");
    let Some(name) = names["data"]
        .get(0)
        .and_then(Value::as_array)
        .and_then(|names| {
            names.iter().filter_map(Value::as_str).find(|name| {
                name.strip_prefix(&prefix).is_some_and(|generation| {
                    !generation.is_empty() && generation.bytes().all(|c| c.is_ascii_digit())
                })
            })
        })
    else {
        return false;
    };
    let Some(owner) = call(&[
        "call",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        "s",
        name,
    ]) else {
        return false;
    };
    let Some(owner) = owner["data"]
        .as_array()
        .and_then(|data| data.first())
        .and_then(Value::as_str)
    else {
        return false;
    };
    if !call(&[
        "call",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixProcessID",
        "s",
        owner,
    ])
    .is_some_and(|value| value["data"].get(0).and_then(Value::as_u64) == Some(u64::from(pid)))
    {
        return false;
    }
    if !call(&[
        "get-property",
        WATCHER,
        WATCHER_PATH,
        WATCHER,
        "IsStatusNotifierHostRegistered",
    ])
    .is_some_and(|value| value["data"] == true)
    {
        return false;
    }
    let Some(items) = call(&[
        "get-property",
        WATCHER,
        WATCHER_PATH,
        WATCHER,
        "RegisteredStatusNotifierItems",
    ]) else {
        return false;
    };
    items["data"].as_array().is_some_and(|items| {
        items.iter().filter_map(Value::as_str).any(|item| {
            [name, owner]
                .iter()
                .any(|service| item == format!("{service}{ITEM_PATH}") || item == *service)
        })
    })
}

fn busctl(arguments: &[&str]) -> Option<Value> {
    // No shell, no environment mutation, no writes to desktop configuration.
    let mut child = Command::new("busctl")
        .args(["--user", "--json=short", "--timeout=0.3s", "--no-pager"])
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_millis(450);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !success {
        return None;
    }
    // A full pipe times out above instead of allocating unbounded upstream output.
    let mut output = Vec::new();
    child
        .stdout
        .take()?
        .take(65537)
        .read_to_end(&mut output)
        .ok()?;
    if output.len() > 65536 {
        return None;
    }
    serde_json::from_slice(&output).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake(pid: u32, host: bool, items: &[&str], own_name: &str) -> bool {
        probe(pid, |args| match args.last().copied() {
            Some("ListNames") => Some(json!({"type":"as", "data": [[own_name]]})),
            Some("IsStatusNotifierHostRegistered") => Some(json!({"data": host})),
            Some("RegisteredStatusNotifierItems") => Some(json!({"data": items})),
            _ if args.contains(&"GetNameOwner") => Some(json!({"data": [":1.42"]})),
            _ if args.contains(&"GetConnectionUnixProcessID") => Some(json!({"data": [pid]})),
            _ => None,
        })
    }

    #[test]
    fn hide_requires_the_process_item_in_an_active_host() {
        let name = "org.kde.StatusNotifierItem-42-1";
        assert!(fake(
            42,
            true,
            &["org.kde.StatusNotifierItem-42-1/StatusNotifierItem"],
            name
        ));
        assert!(fake(42, true, &[":1.42/StatusNotifierItem"], name));
        assert!(!fake(42, false, &[":1.42/StatusNotifierItem"], name));
        assert!(!fake(42, true, &[":1.99/StatusNotifierItem"], name));
        assert!(!fake(42, true, &[":1.42/OtherItem"], name));
        assert!(!fake(
            42,
            true,
            &[":1.42/StatusNotifierItem"],
            "org.kde.StatusNotifierItem-420-1"
        ));
        assert!(!fake(
            42,
            true,
            &[":1.42/StatusNotifierItem"],
            "org.kde.StatusNotifierItem-42-"
        ));
        assert!(!probe(42, |_| None));
        assert!(!probe(42, |_| Some(json!({"data": "invalid"}))));
    }
}
