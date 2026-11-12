use super::*;
use std::cell::RefCell;

fn binding() -> Binding {
    Binding {
        relay: RelayAddress::parse("https://relay.example.com").unwrap(),
        route: protocol::encode(&[1; 32]),
        host_key: protocol::encode(&[2; 32]),
        device_id: "paired-device".into(),
    }
}
fn saved() -> SavedBindings {
    let mut saved = decode_bindings(None).unwrap();
    saved.bindings.push(binding());
    saved.selected = Some(BindingIdentity::of(&binding()));
    saved.local = Some(true);
    saved.aliases.0.push(HostAlias {
        identity: BindingIdentity::of(&binding()),
        name: "我的主机".into(),
    });
    saved
}
async fn recorded(saved: SavedBindings, target: &Binding, name: &str) -> SavedBindings {
    let written = RefCell::new(None);
    remember_with(saved, target, name, |saved| async {
        written.replace(Some(saved));
        Ok(())
    })
    .await
    .unwrap();
    written.into_inner().unwrap()
}

#[test]
fn missing_hostname_metadata_restores_old_bindings_and_aliases() {
    for document in [
        None,
        Some(serde_json::to_string(&vec![binding()]).unwrap()),
        Some(serde_json::to_string(&saved()).unwrap()),
    ] {
        assert!(
            decode_bindings(document.as_deref())
                .unwrap()
                .host_names
                .is_empty()
        );
    }
    let document = serde_json::to_string(&saved()).unwrap();
    assert!(!document.contains("host_names"));
    assert_eq!(
        decode_bindings(Some(&document))
            .unwrap()
            .aliases
            .get(&binding()),
        Some("我的主机")
    );
}

#[tokio::test]
async fn hostname_roundtrip_preserves_binding_selection_alias_and_auto_connect() {
    let target = binding();
    let saved = recorded(saved(), &target, "  开发机.local  ").await;
    let saved = decode_bindings(Some(&serde_json::to_string(&saved).unwrap())).unwrap();
    assert_eq!(saved.host_names.get(&target), Some("开发机.local"));
    assert_eq!(saved.aliases.get(&target), Some("我的主机"));
    assert_eq!(saved.bindings, vec![target.clone()]);
    assert!(saved.selected.unwrap().matches(&target));
    assert!(!saved.auto_connect);
    assert_eq!(saved.local, Some(true));
    let debug = format!("{:?}", saved.host_names);
    assert!(!debug.contains(&target.route) && !debug.contains(&target.host_key));
}

#[tokio::test]
async fn hostname_is_scoped_by_relay_route_and_key_and_survives_device_repair() {
    let target = binding();
    let mut saved = recorded(saved(), &target, "server-one").await;
    for changed in ["relay", "route", "key"] {
        let mut other = target.clone();
        match changed {
            "relay" => other.relay = RelayAddress::parse("https://other.example.com").unwrap(),
            "route" => other.route = protocol::encode(&[3; 32]),
            _ => other.host_key = protocol::encode(&[3; 32]),
        }
        assert_eq!(saved.host_names.get(&other), None, "{changed}");
    }
    let mut repaired = target.clone();
    repaired.device_id = "repaired-device".into();
    saved.bindings = remember_in(saved.bindings, repaired.clone()).unwrap();
    saved.host_names.retain(&saved.bindings);
    assert_eq!(saved.host_names.get(&repaired), Some("server-one"));
    let mut rotated = repaired.clone();
    rotated.host_key = protocol::encode(&[3; 32]);
    saved.bindings = remember_in(saved.bindings, rotated.clone()).unwrap();
    saved.host_names.retain(&saved.bindings);
    assert!(saved.host_names.is_empty());
    let mut saved = recorded(saved, &rotated, "server-two").await;
    saved.bindings = forget_from(saved.bindings, &rotated);
    saved.host_names.retain(&saved.bindings);
    assert!(saved.host_names.is_empty());
}

#[tokio::test]
async fn hostname_storage_error_is_not_confirmed_and_unchanged_metadata_does_not_write() {
    let original = recorded(saved(), &binding(), "server-one").await;
    let result = remember_with(original, &binding(), "server-one", |_| async {
        panic!("unchanged hostname must not write")
    })
    .await
    .unwrap();
    assert_eq!(result.get(&binding()), Some("server-one"));
    let result = remember_with(
        recorded(saved(), &binding(), "server-one").await,
        &binding(),
        "server-two",
        |_| async { anyhow::bail!("unconfirmed write") },
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn invalid_or_stale_hostname_metadata_never_writes() {
    for name in [
        "",
        "  ",
        "host\nname",
        "host\0name",
        "host\u{2028}name",
        &"x".repeat(256),
    ] {
        assert!(
            remember_with(saved(), &binding(), name, |_| async {
                panic!("invalid name must not write")
            })
            .await
            .is_err()
        );
    }
    let mut stale = binding();
    stale.device_id = "old-device".into();
    assert!(
        remember_with(saved(), &stale, "server", |_| async {
            panic!("stale binding must not write")
        })
        .await
        .is_err()
    );
}

#[tokio::test]
async fn damaged_cached_names_are_explicit_storage_errors() {
    let saved = recorded(saved(), &binding(), "server").await;
    let mut document = serde_json::to_value(&saved).unwrap();
    document["host_names"][0]["name"] = "server\n".into();
    assert!(decode_bindings(Some(&document.to_string())).is_err());
    document["host_names"][0]["name"] = "server".into();
    let duplicate = document["host_names"][0].clone();
    document["host_names"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert!(decode_bindings(Some(&document.to_string())).is_err());
}

#[tokio::test]
async fn offline_alias_rename_and_clear_preserve_reported_hostname_cache() {
    let target = binding();
    let original = recorded(saved(), &target, "server.local").await;
    let written = RefCell::new(None);
    rename_with(original, &target, "离线改名", |saved| async {
        written.replace(Some(saved));
        Ok(())
    })
    .await
    .unwrap();
    let original = written.take().unwrap();
    assert_eq!(original.host_names.get(&target), Some("server.local"));
    assert_eq!(original.aliases.get(&target), Some("离线改名"));
    rename_with(original, &target, "", |saved| async {
        written.replace(Some(saved));
        Ok(())
    })
    .await
    .unwrap();
    let saved = written.into_inner().unwrap();
    assert_eq!(saved.host_names.get(&target), Some("server.local"));
    assert_eq!(saved.aliases.get(&target), None);
    assert!(!saved.auto_connect);
    assert_eq!(saved.local, Some(true));
    assert!(saved.selected.unwrap().matches(&target));
}
