//! Server-reported names are display metadata, separate from binding identity and aliases.
use super::*;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HostNames(Vec<HostAlias>);

impl std::fmt::Debug for HostNames {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostNames")
            .field("count", &self.0.len())
            .finish()
    }
}

impl HostNames {
    pub(crate) fn get(&self, binding: &Binding) -> Option<&str> {
        self.0
            .iter()
            .find(|entry| entry.identity.matches(binding))
            .map(|entry| entry.name.as_str())
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn retain(&mut self, bindings: &[Binding]) {
        self.0.retain(|entry| {
            bindings
                .iter()
                .any(|binding| entry.identity.matches(binding))
        });
    }
    pub(super) fn validate(&self, bindings: &[Binding]) -> Result<()> {
        ensure!(self.0.len() <= protocol::MAX_DEVICES, "已保存的主机名过多");
        for (index, entry) in self.0.iter().enumerate() {
            ensure!(
                normalize_host_name(&entry.name)? == entry.name,
                "已保存的主机名无效"
            );
            ensure!(
                bindings
                    .iter()
                    .any(|binding| entry.identity.matches(binding))
                    && !self.0[..index]
                        .iter()
                        .any(|previous| previous.identity == entry.identity),
                "已保存的主机名身份无效"
            );
        }
        Ok(())
    }
}

pub(crate) fn normalize_host_name(name: &str) -> Result<String> {
    ensure!(
        !name.chars().any(|character| character.is_control())
            && !name.contains(['\u{2028}', '\u{2029}']),
        "远程主机名包含控制字符"
    );
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.len() <= 255,
        "远程主机名为空或过长"
    );
    Ok(name.to_owned())
}

pub(crate) async fn remember_host_name(binding: &Binding, name: &str) -> Result<HostNames> {
    remember_with(load_bindings().await?, binding, name, |saved| async move {
        save_bindings(&saved).await
    })
    .await
}

async fn remember_with<Save>(
    mut saved: SavedBindings,
    binding: &Binding,
    name: &str,
    persist: impl FnOnce(SavedBindings) -> Save,
) -> Result<HostNames>
where
    Save: std::future::Future<Output = Result<()>>,
{
    validate(binding)?;
    ensure!(
        saved.bindings.contains(binding),
        "该设备配对已变化，请重新选择"
    );
    let name = normalize_host_name(name)?;
    if saved.host_names.get(binding) == Some(name.as_str()) {
        return Ok(saved.host_names);
    }
    saved
        .host_names
        .0
        .retain(|entry| !entry.identity.matches(binding));
    saved.host_names.0.push(HostAlias {
        identity: BindingIdentity::of(binding),
        name,
    });
    let names = saved.host_names.clone();
    persist(saved).await?;
    Ok(names)
}

#[cfg(test)]
#[path = "reported_hosts/tests.rs"]
mod tests;
