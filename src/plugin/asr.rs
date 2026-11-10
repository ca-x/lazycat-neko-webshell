//! Registered ASR providers share the existing voice upload path.
use super::PluginHost;
use neko_plugin::PluginError;

/// Whether the built-in manifest registers an ASR provider.
/// Saved metadata or a caller-supplied plugin ID cannot grant this capability.
pub(crate) fn registered(plugin_id: &str) -> bool {
    super::catalog::ENTRIES
        .iter()
        .find(|entry| entry.id == plugin_id)
        .is_some_and(|entry| {
            serde_json::from_str::<serde_json::Value>(entry.json)
                .ok()
                .is_some_and(|manifest| manifest["metadata"]["asrProvider"] == "true")
        })
}

impl PluginHost {
    pub(crate) fn require_asr_provider(&self, plugin_id: &str) -> Result<(), PluginError> {
        if !registered(plugin_id) {
            return Err(PluginError::invalid_argument(
                "ASR provider is not registered",
            ));
        }
        self.require_enabled(plugin_id)
            .map_err(super::resources::plugin_error)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_declared_asr_plugins_are_providers() {
        assert!(super::registered("wechat-asr"));
        assert!(!super::registered("ai-chat"));
        assert!(!super::registered("file-transfer"));
        assert!(!super::registered("missing"));
    }
}
