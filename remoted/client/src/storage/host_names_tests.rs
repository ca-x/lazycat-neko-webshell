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
    saved
}

async fn renamed(saved: SavedBindings, binding: &Binding, name: &str) -> SavedBindings {
    let persisted = RefCell::new(None);
    rename_with(saved, binding, name, |saved| async {
        persisted.replace(Some(saved));
        Ok(())
    })
    .await
    .unwrap();
    persisted.into_inner().unwrap()
}

#[test]
fn aliases_default_for_missing_legacy_and_existing_envelopes() {
    assert!(decode_bindings(None).unwrap().aliases.is_empty());
    let legacy = serde_json::to_string(&vec![binding()]).unwrap();
    let restored = decode_bindings(Some(&legacy)).unwrap();
    assert!(restored.aliases.is_empty());
    assert!(restored.auto_connect);
    assert!(restored.selected.unwrap().matches(&binding()));
    let envelope = serde_json::to_string(&saved()).unwrap();
    assert!(!envelope.contains("aliases"));
    let restored = decode_bindings(Some(&envelope)).unwrap();
    assert!(restored.aliases.is_empty());
    assert!(!restored.auto_connect);
    assert_eq!(restored.local, Some(true));
}

#[tokio::test]
async fn rename_chinese_roundtrips_without_changing_binding_or_preferences() {
    let target = binding();
    let restored = renamed(saved(), &target, "  开发主机 🐈  ").await;
    let restored = decode_bindings(Some(&serde_json::to_string(&restored).unwrap())).unwrap();
    assert_eq!(restored.aliases.get(&target), Some("开发主机 🐈"));
    assert_eq!(restored.bindings, vec![target.clone()]);
    assert!(restored.bindings.contains(&target));
    assert!(restored.selected.unwrap().matches(&target));
    assert!(!restored.auto_connect);
    assert_eq!(restored.local, Some(true));
    let debug = format!("{:?}", restored.aliases);
    assert!(!debug.contains(&target.host_key));
    assert!(!debug.contains(&target.route));
}

#[tokio::test]
async fn rename_unselected_host_does_not_select_it_or_enable_auto_connect() {
    let mut saved = saved();
    let mut selected = binding();
    selected.route = protocol::encode(&[3; 32]);
    saved.bindings.push(selected.clone());
    saved.selected = Some(BindingIdentity::of(&selected));
    let saved = renamed(saved, &binding(), "另一台主机").await;
    assert_eq!(saved.aliases.get(&binding()), Some("另一台主机"));
    assert!(saved.selected.unwrap().matches(&selected));
    assert!(!saved.auto_connect);
    assert_eq!(saved.local, Some(true));
}

#[tokio::test]
async fn rename_ambiguous_publication_never_returns_a_confirmed_alias() {
    let published = RefCell::new(None);
    let result = rename_with(saved(), &binding(), "可能已写入", |saved| {
        published.replace(Some(serde_json::to_string(&saved).unwrap()));
        async { Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into()) }
    })
    .await;
    assert!(result.is_err());
    let document = published.into_inner().unwrap();
    let stored = decode_bindings(Some(&document)).unwrap();
    assert_eq!(stored.aliases.get(&binding()), Some("可能已写入"));
}

#[tokio::test]
async fn rename_replaces_and_clears_only_the_exact_host_alias() {
    let target = binding();
    let mut other = target.clone();
    other.relay = RelayAddress::parse("https://other.example.com").unwrap();
    let mut original = saved();
    original.bindings.push(other.clone());
    let saved = renamed(original, &other, "另一台").await;
    let saved = renamed(saved, &target, "开发主机").await;
    let saved = renamed(saved, &target, "新名称").await;
    assert_eq!(saved.aliases.0.len(), 2);
    assert_eq!(saved.aliases.get(&target), Some("新名称"));
    let saved = renamed(saved, &target, "　 ").await;
    assert_eq!(saved.aliases.get(&target), None);
    assert_eq!(saved.aliases.get(&other), Some("另一台"));
}

#[tokio::test]
async fn rename_rejects_stale_pairing_before_persistence() {
    for changed in ["host", "relay", "route", "device"] {
        let mut stale = binding();
        match changed {
            "host" => stale.host_key = protocol::encode(&[9; 32]),
            "relay" => stale.relay = RelayAddress::parse("https://other.example.com").unwrap(),
            "route" => stale.route = protocol::encode(&[9; 32]),
            _ => stale.device_id = "stale-device".into(),
        }
        let result = rename_with(saved(), &stale, "开发主机", |_| async {
            panic!("stale identities must not write storage")
        })
        .await;
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(!message.contains(&stale.host_key));
        assert!(!message.contains(&stale.route));
    }
}

#[test]
fn aliases_validate_utf8_bounds_and_reject_controls() {
    assert_eq!(normalize_host_alias("  中文 🐈　").unwrap(), "中文 🐈");
    assert!(normalize_host_alias(&"a".repeat(128)).is_ok());
    assert!(normalize_host_alias(&"主".repeat(42)).is_ok());
    assert!(normalize_host_alias(&"主".repeat(43)).is_err());
    for invalid in ["\n主机", "主\t机", "主机\r", "主\0机", "主\u{2028}机"] {
        assert!(normalize_host_alias(invalid).is_err());
    }
}

#[tokio::test]
async fn rename_invalid_alias_does_not_write_or_report_a_saved_value() {
    for invalid in ["a".repeat(129), "主机\n".into()] {
        assert!(
            rename_with(saved(), &binding(), &invalid, |_| async {
                panic!("invalid aliases must not write storage")
            })
            .await
            .is_err()
        );
    }
    let result = rename_with(saved(), &binding(), "开发主机", |_| async {
        Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into())
    })
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn aliases_follow_exact_identity_through_repair_rotation_and_forgetting() {
    let target = binding();
    let mut saved = renamed(saved(), &target, "开发主机").await;
    let mut repaired = target.clone();
    repaired.device_id = "repaired-device".into();
    saved.bindings = remember_in(saved.bindings, repaired.clone()).unwrap();
    saved.aliases.retain(&saved.bindings);
    assert_eq!(saved.aliases.get(&repaired), Some("开发主机"));
    let mut rotated = repaired.clone();
    rotated.host_key = protocol::encode(&[9; 32]);
    assert_eq!(saved.aliases.get(&rotated), None);
    saved.bindings = remember_in(saved.bindings, rotated.clone()).unwrap();
    saved.aliases.retain(&saved.bindings);
    assert!(saved.aliases.is_empty());
    let mut saved = renamed(saved, &rotated, "换钥后").await;
    saved.bindings = forget_from(saved.bindings, &target);
    saved.aliases.retain(&saved.bindings);
    assert_eq!(saved.aliases.get(&rotated), Some("换钥后"));
    saved.bindings = forget_from(saved.bindings, &rotated);
    saved.aliases.retain(&saved.bindings);
    assert!(saved.aliases.is_empty());
}

#[tokio::test]
async fn damaged_alias_metadata_is_not_silently_reset() {
    let saved = renamed(saved(), &binding(), "开发主机").await;
    let mut document = serde_json::to_value(&saved).unwrap();
    document["aliases"][0]["name"] = "主机\n".into();
    assert!(decode_bindings(Some(&document.to_string())).is_err());
    document["aliases"][0]["name"] = "开发主机".into();
    let duplicate = document["aliases"][0].clone();
    document["aliases"].as_array_mut().unwrap().push(duplicate);
    assert!(decode_bindings(Some(&document.to_string())).is_err());
}
