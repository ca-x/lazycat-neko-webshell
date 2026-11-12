use anyhow::{Context, Result, ensure};
use neko_remoted_protocol::{self as protocol, RelayAddress, crypto::Identity};
use serde::{Deserialize, Serialize};

#[path = "storage/reported_hosts.rs"]
mod reported_hosts;
pub use reported_hosts::HostNames;
pub(crate) use reported_hosts::{normalize_host_name, remember_host_name};

const IDENTITY: &str = "remoted.identity.v1";
const BINDINGS: &str = "remoted.bindings.v1";
const LAST_RELAY: &str = "remoted.last-relay.v1";

pub async fn last_relay() -> Result<Option<RelayAddress>> {
    parse_relay_preference(crate::platform::secure_get(LAST_RELAY).await?.as_deref())
}

fn parse_relay_preference(value: Option<&str>) -> Result<Option<RelayAddress>> {
    value
        .map(RelayAddress::parse)
        .transpose()
        .context("已保存的 Relay 地址无效，请重新填写")
}

pub async fn remember_relay(relay: &RelayAddress) -> Result<()> {
    crate::platform::secure_set(LAST_RELAY, relay.origin()).await
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub relay: RelayAddress,
    pub route: String,
    pub host_key: String,
    pub device_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Secret {
    private: String,
    public: String,
}

pub async fn identity() -> Result<Identity> {
    if let Some(value) = crate::platform::secure_get(IDENTITY).await? {
        let saved: Secret =
            serde_json::from_str(&value).context("设备身份存储损坏；请勿覆盖现有身份")?;
        return Ok(Identity::restore(
            protocol::decode32(&saved.private)?,
            protocol::decode32(&saved.public)?,
        ));
    }
    let identity = Identity::generate()?;
    let (private, public) = identity.protected_export();
    let value = serde_json::to_string(&Secret {
        private: protocol::encode(private),
        public: protocol::encode(public),
    })?;
    crate::platform::secure_set(IDENTITY, &value).await?;
    Ok(identity)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BindingIdentity {
    relay: RelayAddress,
    route: String,
    host_key: String,
}
impl BindingIdentity {
    fn of(binding: &Binding) -> Self {
        Self {
            relay: binding.relay.clone(),
            route: binding.route.clone(),
            host_key: binding.host_key.clone(),
        }
    }
    fn matches(&self, binding: &Binding) -> bool {
        self.relay == binding.relay
            && self.route == binding.route
            && self.host_key == binding.host_key
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HostAliases(Vec<HostAlias>);

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct HostAlias {
    identity: BindingIdentity,
    name: String,
}

impl std::fmt::Debug for HostAliases {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostAliases")
            .field("count", &self.0.len())
            .finish()
    }
}

impl HostAliases {
    pub(crate) fn get(&self, binding: &Binding) -> Option<&str> {
        self.0
            .iter()
            .find(|alias| alias.identity.matches(binding))
            .map(|alias| alias.name.as_str())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn retain(&mut self, bindings: &[Binding]) {
        self.0.retain(|alias| {
            bindings
                .iter()
                .any(|binding| alias.identity.matches(binding))
        });
    }
}

pub(crate) fn normalize_host_alias(alias: &str) -> Result<String> {
    ensure!(
        !alias.chars().any(|character| character.is_control())
            && !alias.contains(['\u{2028}', '\u{2029}']),
        "主机别名不能包含换行或控制字符"
    );
    let alias = alias.trim();
    ensure!(alias.len() <= 128, "主机别名最多为 128 字节");
    Ok(alias.to_owned())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedBindings {
    version: u8,
    bindings: Vec<Binding>,
    selected: Option<BindingIdentity>,
    auto_connect: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local: Option<bool>,
    #[serde(default, skip_serializing_if = "HostAliases::is_empty")]
    aliases: HostAliases,
    #[serde(default, skip_serializing_if = "HostNames::is_empty")]
    host_names: HostNames,
}

pub(crate) struct Preferences {
    pub bindings: Vec<Binding>,
    pub selected: Option<Binding>,
    pub auto_connect: bool,
    pub aliases: HostAliases,
    pub host_names: HostNames,
}

fn decode_bindings(value: Option<&str>) -> Result<SavedBindings> {
    let Some(value) = value else {
        return Ok(SavedBindings {
            version: 1,
            bindings: Vec::new(),
            selected: None,
            auto_connect: false,
            local: None,
            aliases: HostAliases::default(),
            host_names: HostNames::default(),
        });
    };
    let saved: SavedBindings = if value.trim_start().starts_with('[') {
        let bindings: Vec<Binding> = serde_json::from_str(value).context("已配对设备存储损坏")?;
        // Only a single legacy binding has an unambiguous most-recent identity.
        let selected = (bindings.len() == 1).then(|| BindingIdentity::of(&bindings[0]));
        let auto_connect = selected.is_some();
        SavedBindings {
            version: 1,
            bindings,
            selected,
            auto_connect,
            local: None,
            aliases: HostAliases::default(),
            host_names: HostNames::default(),
        }
    } else {
        serde_json::from_str(value).context("已配对设备和连接偏好存储损坏")?
    };
    ensure!(saved.version == 1, "不支持的设备偏好版本；未覆盖现有存储");
    ensure!(
        saved.bindings.len() <= protocol::MAX_DEVICES,
        "已配对设备过多"
    );
    for binding in &saved.bindings {
        validate(binding)?;
    }
    ensure!(
        saved.aliases.0.len() <= protocol::MAX_DEVICES,
        "主机别名过多"
    );
    for (index, alias) in saved.aliases.0.iter().enumerate() {
        ensure!(
            !alias.name.is_empty() && normalize_host_alias(&alias.name)? == alias.name,
            "已保存的主机别名无效"
        );
        ensure!(
            saved
                .bindings
                .iter()
                .any(|binding| alias.identity.matches(binding))
                && !saved.aliases.0[..index]
                    .iter()
                    .any(|previous| previous.identity == alias.identity),
            "已保存的主机别名身份无效"
        );
    }
    saved.host_names.validate(&saved.bindings)?;
    if let Some(selected) = &saved.selected {
        protocol::decode32(&selected.route).context("上次选择的设备 route 损坏")?;
        protocol::decode32(&selected.host_key).context("上次选择的设备公钥损坏")?;
    }
    Ok(saved)
}

async fn load_bindings() -> Result<SavedBindings> {
    decode_bindings(crate::platform::secure_get(BINDINGS).await?.as_deref())
}
async fn save_bindings(saved: &SavedBindings) -> Result<()> {
    crate::platform::secure_set(BINDINGS, &serde_json::to_string(saved)?).await
}
pub(crate) async fn preferences() -> Result<Preferences> {
    let saved = load_bindings().await?;
    let selected = saved
        .selected
        .as_ref()
        .and_then(|id| saved.bindings.iter().find(|b| id.matches(b)))
        .cloned();
    Ok(Preferences {
        auto_connect: saved.auto_connect && selected.is_some(),
        bindings: saved.bindings,
        selected,
        aliases: saved.aliases,
        host_names: saved.host_names,
    })
}

pub(crate) async fn rename_binding(binding: &Binding, alias: &str) -> Result<HostAliases> {
    rename_with(load_bindings().await?, binding, alias, |saved| async move {
        save_bindings(&saved).await
    })
    .await
}

async fn rename_with<Save>(
    mut saved: SavedBindings,
    binding: &Binding,
    alias: &str,
    persist: impl FnOnce(SavedBindings) -> Save,
) -> Result<HostAliases>
where
    Save: std::future::Future<Output = Result<()>>,
{
    validate(binding)?;
    ensure!(
        saved.bindings.contains(binding),
        "该设备配对已变化，请重新选择"
    );
    let name = normalize_host_alias(alias)?;
    saved
        .aliases
        .0
        .retain(|alias| !alias.identity.matches(binding));
    if !name.is_empty() {
        saved.aliases.0.push(HostAlias {
            identity: BindingIdentity::of(binding),
            name,
        });
    }
    let aliases = saved.aliases.clone();
    persist(saved).await?;
    Ok(aliases)
}

/// Persists only an existing exact host identity; never creates or migrates a binding.
pub(crate) async fn select_binding(binding: &Binding) -> Result<()> {
    let mut saved = load_bindings().await?;
    validate(binding)?;
    ensure!(
        saved.bindings.contains(binding),
        "该设备配对已变化，请重新选择"
    );
    saved.selected = Some(BindingIdentity::of(binding));
    saved.auto_connect = true;
    saved.local = Some(false);
    save_bindings(&saved).await
}
pub(crate) async fn stop_auto_connect() -> Result<()> {
    let mut saved = load_bindings().await?;
    saved.auto_connect = false;
    save_bindings(&saved).await
}

pub(crate) fn validate(binding: &Binding) -> Result<()> {
    protocol::decode32(&binding.route)?;
    protocol::decode32(&binding.host_key)?;
    ensure!(
        !binding.device_id.is_empty() && binding.device_id.len() <= 256,
        "设备标识无效"
    );
    Ok(())
}

pub async fn remember(binding: Binding) -> Result<Vec<Binding>> {
    let mut saved = load_bindings().await?;
    saved.bindings = remember_in(saved.bindings, binding.clone())?;
    saved.aliases.retain(&saved.bindings);
    saved.host_names.retain(&saved.bindings);
    saved.selected = Some(BindingIdentity::of(&binding));
    saved.auto_connect = true;
    saved.local = Some(false);
    save_bindings(&saved).await?;
    Ok(saved.bindings)
}

fn remember_in(mut saved: Vec<Binding>, binding: Binding) -> Result<Vec<Binding>> {
    validate(&binding)?;
    saved.retain(|item| !(item.relay == binding.relay && item.route == binding.route));
    ensure!(saved.len() < protocol::MAX_DEVICES, "已配对设备已达上限");
    saved.push(binding);
    Ok(saved)
}

pub async fn forget(binding: &Binding) -> Result<Vec<Binding>> {
    let mut saved = load_bindings().await?;
    saved.bindings = forget_from(saved.bindings, binding);
    saved.aliases.retain(&saved.bindings);
    saved.host_names.retain(&saved.bindings);
    if saved
        .selected
        .as_ref()
        .is_some_and(|id| id.matches(binding))
    {
        saved.selected = None;
        saved.auto_connect = false;
    }
    save_bindings(&saved).await?;
    Ok(saved.bindings)
}

fn forget_from(mut saved: Vec<Binding>, binding: &Binding) -> Vec<Binding> {
    saved.retain(|item| {
        !(item.relay == binding.relay
            && item.route == binding.route
            && item.host_key == binding.host_key)
    });
    saved
}

/// Returns the selected workspace without conflating it with hosting configuration.
#[cfg(not(target_os = "android"))]
pub(crate) async fn local_workspace() -> Result<Option<bool>> {
    Ok(load_bindings().await?.local)
}

/// Saves the local selection together with the existing remote reconnect preference.
#[cfg(not(target_os = "android"))]
pub(crate) async fn select_local_workspace() -> Result<()> {
    let mut saved = load_bindings().await?;
    saved.local = Some(true);
    save_bindings(&saved).await
}

#[cfg(test)]
#[path = "storage/host_names_tests.rs"]
mod host_names_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_binding_is_not_a_reconnect_target() {
        let binding = Binding {
            relay: RelayAddress::parse("https://relay.example.com").unwrap(),
            route: "bad".into(),
            host_key: protocol::encode(&[1; 32]),
            device_id: "device".into(),
        };
        assert!(validate(&binding).is_err());
    }

    #[test]
    fn saved_relay_prefill_is_canonical_and_missing_is_unconfigured() {
        assert_eq!(parse_relay_preference(None).unwrap(), None);
        assert_eq!(
            parse_relay_preference(Some("https://relay.example.com"))
                .unwrap()
                .unwrap()
                .origin(),
            "wss://relay.example.com"
        );
    }

    #[test]
    fn malformed_or_insecure_saved_relay_is_an_explicit_error() {
        for value in [
            "",
            "not a relay",
            "http://relay.example.com",
            "https://relay.example.com/path",
            "https://user:secret@relay.example.com",
        ] {
            assert!(
                parse_relay_preference(Some(value)).is_err(),
                "preference {value}"
            );
        }
    }

    fn binding(index: u8) -> Binding {
        Binding {
            relay: RelayAddress::parse("https://relay.example.com").unwrap(),
            route: protocol::encode(&[index; 32]),
            host_key: protocol::encode(&[2; 32]),
            device_id: format!("device-{index}"),
        }
    }

    #[test]
    fn local_forget_matches_relay_route_and_host_key_without_deleting_rotated_binding() {
        let original = binding(1);
        let mut rotated = original.clone();
        rotated.host_key = protocol::encode(&[9; 32]);
        let mut other_relay = original.clone();
        other_relay.relay = RelayAddress::parse("https://other.example.com").unwrap();
        let other_route = binding(2);
        let saved = forget_from(
            vec![
                original.clone(),
                rotated.clone(),
                other_relay.clone(),
                other_route.clone(),
            ],
            &original,
        );
        assert_eq!(saved, vec![rotated, other_relay, other_route]);
        assert_eq!(forget_from(saved.clone(), &original), saved);
    }

    #[test]
    fn forgetting_one_binding_recovers_the_full_device_limit() {
        let saved = (0..32).map(binding).collect::<Vec<_>>();
        assert!(remember_in(saved.clone(), binding(32)).is_err());
        let saved = forget_from(saved, &binding(7));
        let saved = remember_in(saved, binding(32)).unwrap();
        assert_eq!(saved.len(), 32);
        assert!(!saved.iter().any(|item| item.route == binding(7).route));
    }
    #[test]
    fn legacy_single_binding_migrates_but_multiple_bindings_do_not_guess_a_host() {
        let one =
            decode_bindings(Some(&serde_json::to_string(&vec![binding(1)]).unwrap())).unwrap();
        assert!(one.auto_connect && one.selected.as_ref().unwrap().matches(&binding(1)));
        let several = decode_bindings(Some(
            &serde_json::to_string(&vec![binding(1), binding(2)]).unwrap(),
        ))
        .unwrap();
        assert!(!several.auto_connect && several.selected.is_none());
    }
    #[test]
    fn exact_host_preference_and_manual_stop_survive_persistence_roundtrip() {
        let old = binding(1);
        let mut rotated = old.clone();
        rotated.host_key = protocol::encode(&[9; 32]);
        let mut saved = SavedBindings {
            version: 1,
            bindings: vec![old.clone(), rotated.clone()],
            selected: Some(BindingIdentity::of(&old)),
            auto_connect: true,
            local: None,
            aliases: HostAliases::default(),
            host_names: HostNames::default(),
        };
        let restored = decode_bindings(Some(&serde_json::to_string(&saved).unwrap())).unwrap();
        assert!(restored.selected.as_ref().unwrap().matches(&old));
        assert!(!restored.selected.as_ref().unwrap().matches(&rotated));
        saved.auto_connect = false;
        let stopped = decode_bindings(Some(&serde_json::to_string(&saved).unwrap())).unwrap();
        assert!(!stopped.auto_connect && stopped.selected.is_some());
    }
}
