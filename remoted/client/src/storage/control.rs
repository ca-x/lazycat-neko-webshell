//! Acknowledged, per-device and per-terminal control preferences.
use super::*;
use tokio::sync::Mutex;

pub(crate) const KEY: &str = "remoted.terminal-control.v1";
const LIMIT: usize = 256;
static WRITER: Mutex<()> = Mutex::const_new(());

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Choices {
    entries: Vec<Choice>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    binding: Binding,
    grant: String,
    pane: Option<String>,
    take: bool,
}

async fn read() -> Result<Choices> {
    let value = crate::platform::secure_get(KEY).await?;
    let choices: Choices = value
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    ensure!(choices.entries.len() <= LIMIT, "控制偏好超过存储上限");
    Ok(choices)
}

pub(crate) async fn load(binding: &Binding) -> Result<Vec<(String, Option<String>, bool)>> {
    Ok(read()
        .await?
        .entries
        .into_iter()
        .filter(|choice| choice.binding == *binding)
        .map(|choice| (choice.grant, choice.pane, choice.take))
        .collect())
}

pub(crate) async fn remember(
    binding: &Binding,
    grant: &str,
    pane: Option<&str>,
    take: bool,
) -> Result<()> {
    let _guard = WRITER.lock().await;
    let mut choices = read().await?;
    choices.entries.retain(|choice| {
        !(choice.binding == *binding && choice.grant == grant && choice.pane.as_deref() == pane)
    });
    if choices.entries.len() == LIMIT {
        choices.entries.remove(0);
    }
    choices.entries.push(Choice {
        binding: binding.clone(),
        grant: grant.to_owned(),
        pane: pane.map(str::to_owned),
        take,
    });
    // The native credential store has a 64 KiB document limit on every platform.
    let mut encoded = serde_json::to_string(&choices)?;
    while encoded.len() > 60 * 1024 && choices.entries.len() > 1 {
        choices.entries.remove(0);
        encoded = serde_json::to_string(&choices)?;
    }
    ensure!(encoded.len() <= 60 * 1024, "终端控制偏好超过存储容量");
    crate::platform::secure_set(KEY, &encoded).await
}

/// Device removal also removes its saved terminal preferences.
pub(crate) async fn forget(binding: &Binding) -> Result<()> {
    let _guard = WRITER.lock().await;
    let mut choices = read().await?;
    let before = choices.entries.len();
    choices.entries.retain(|choice| choice.binding != *binding);
    if choices.entries.len() != before {
        crate::platform::secure_set(KEY, &serde_json::to_string(&choices)?).await?;
    }
    Ok(())
}
