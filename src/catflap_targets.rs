//! Catflap's typed discovery contract. MicroVM identities never become PTY selectors.
use crate::{
    instance_discovery::Discovery,
    proto::lazycat::webshell::v1::{Instance, InstanceKind},
    validation::validate_selector,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Envelope {
    version: u32,
    targets: Vec<serde_json::Value>,
    sources: Vec<Source>,
}
#[derive(Deserialize)]
struct Source {
    kind: String,
    state: String,
}
#[derive(Deserialize)]
struct Target {
    kind: String,
    name: String,
    status: String,
    capabilities: Capabilities,
    #[serde(default)]
    selector: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    id: String,
}
#[derive(Deserialize)]
struct Capabilities {
    terminal: bool,
}

pub(crate) fn parse(body: &[u8]) -> Result<Discovery, String> {
    let envelope: Envelope =
        serde_json::from_slice(body).map_err(|_| "invalid Catflap targets JSON")?;
    if envelope.version != 1 {
        return Err("unsupported Catflap targets version".into());
    }
    let mut discovery = Discovery::default();
    for source in &envelope.sources {
        if matches!(source.kind.as_str(), "lightos" | "smolvm") && source.state != "ok" {
            discovery
                .warnings
                .push(format!("Catflap {} discovery unavailable", source.kind));
        }
    }
    for value in envelope.targets {
        let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(kind, "lightos" | "smolvm") {
            continue;
        }
        let target: Target = serde_json::from_value(value).map_err(|_| "invalid Catflap target")?;
        // Do not use records from a source that failed its fresh state/ACL lookup.
        if !envelope
            .sources
            .iter()
            .any(|s| s.kind == target.kind && s.state == "ok")
        {
            continue;
        }
        if target.kind == "lightos" {
            validate_selector(&target.selector).map_err(|_| "invalid Catflap LightOS selector")?;
            let (_, owner) = target
                .selector
                .split_once('@')
                .ok_or("invalid Catflap LightOS selector")?;
            discovery.instances.push(Instance {
                kind: Some(InstanceKind::INSTANCE_KIND_LIGHTOS.into()),
                owner_deploy_id: Some(owner.to_owned()),
                terminal_available: Some(
                    target.capabilities.terminal
                        && target.status == "running"
                        && !target.username.trim().is_empty(),
                ),
                selector: Some(target.selector),
                name: Some(target.name),
                username: Some(target.username),
                status: Some(target.status),
                ..Default::default()
            });
        } else {
            uuid::Uuid::parse_str(&target.id).map_err(|_| "invalid Catflap MicroVM id")?;
            discovery.instances.push(Instance {
                kind: Some(InstanceKind::INSTANCE_KIND_SMOLVM.into()),
                target_id: Some(target.id),
                name: Some(target.name),
                status: Some(target.status.clone()),
                terminal_available: Some(
                    target.capabilities.terminal && target.status == "running",
                ),
                ..Default::default()
            });
        }
    }
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn envelope() -> serde_json::Value {
        serde_json::json!({"version":1,"targets":[
            {"kind":"lightos","selector":"work@actual-owner","name":"work","username":"alice","status":"running","managed":true,"capabilities":{"terminal":true,"desktop":false}},
            {"kind":"smolvm","id":"12345678-1234-4234-8234-123456789abc","name":"vm","status":"running","managed":true,"capabilities":{"terminal":false,"desktop":false}}
        ],"sources":[{"kind":"lightos","state":"ok","code":null},{"kind":"smolvm","state":"ok","code":null}]})
    }
    fn read(value: &serde_json::Value) -> Result<Discovery, String> {
        parse(&serde_json::to_vec(value).unwrap())
    }
    #[test]
    fn preserves_typed_identity_and_keeps_microvm_out_of_terminal_transport() {
        let output = read(&envelope()).unwrap();
        assert_eq!(output.instances.len(), 2);
        assert_eq!(
            output.instances[0].selector.as_deref(),
            Some("work@actual-owner")
        );
        assert_eq!(output.instances[0].username.as_deref(), Some("alice"));
        assert_eq!(output.instances[0].terminal_available, Some(true));
        let vm = &output.instances[1];
        assert_eq!(
            vm.target_id.as_deref(),
            Some("12345678-1234-4234-8234-123456789abc")
        );
        assert!(vm.selector.is_none());
        assert!(vm.username.is_none());
        assert!(vm.owner_deploy_id.is_none());
        assert_eq!(vm.terminal_available, Some(false));
    }
    #[test]
    fn unavailable_sources_keep_other_targets_and_ignore_untrusted_codes() {
        let mut value = envelope();
        value["sources"][0]["state"] = "unavailable".into();
        value["sources"][0]["code"] = "secret-or-future-code".into();
        let result = read(&value).unwrap();
        assert_eq!(result.instances.len(), 1);
        assert_eq!(result.warnings, ["Catflap lightos discovery unavailable"]);
    }
    #[test]
    fn capabilities_are_conservative_and_unknown_kinds_are_ignored() {
        let mut value = envelope();
        value["targets"][0]["username"] = "".into();
        value["targets"][1]["capabilities"]["terminal"] = true.into();
        value["targets"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind":"future"}));
        let output = read(&value).unwrap();
        assert_eq!(output.instances.len(), 2);
        assert_eq!(output.instances[0].terminal_available, Some(false));
        assert_eq!(output.instances[1].terminal_available, Some(true));
        value["targets"][1]["id"] = "not-a-uuid".into();
        assert!(read(&value).is_err());
        value["version"] = 2.into();
        assert!(read(&value).is_err());
    }
}
