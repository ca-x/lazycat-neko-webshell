use crate::{
    controller::Model,
    preferences::{tr, tr_format},
    storage::Binding,
};

pub(crate) fn fingerprint(binding: &Binding) -> String {
    neko_remoted_protocol::decode32(&binding.host_key)
        .map(|key| {
            neko_remoted_protocol::fingerprint(&key)
                .chars()
                .take(12)
                .collect()
        })
        .unwrap_or_else(|_| tr("指纹损坏").to_string())
}

pub(crate) fn display_name(model: &Model, binding: &Binding) -> String {
    model
        .aliases
        .get(binding)
        .or_else(|| {
            (model.active_binding.as_ref() == Some(binding) && !model.remote_host_name.is_empty())
                .then_some(model.remote_host_name.as_str())
        })
        .or_else(|| model.host_names.get(binding))
        .map(str::to_owned)
        .unwrap_or_else(|| tr_format("远程主机 · {}", &[&fingerprint(binding)]).to_string())
}

pub(crate) fn active_name(model: &Model) -> String {
    if model.local {
        tr("本机").to_string()
    } else if let Some(binding) = &model.active_binding {
        display_name(model, binding)
    } else {
        tr("远程主机").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{HostAliases, HostNames};
    use nsh_core::{element::Root, prelude::*};
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;

    fn binding() -> Binding {
        Binding {
            relay: neko_remoted_protocol::RelayAddress::parse("https://relay.example.com").unwrap(),
            route: neko_remoted_protocol::encode(&[1; 32]),
            host_key: neko_remoted_protocol::encode(&[2; 32]),
            device_id: "device".into(),
        }
    }
    fn names(target: &Binding, name: &str) -> serde_json::Value {
        serde_json::json!([{"identity": {"relay": target.relay, "route": target.route, "host_key": target.host_key}, "name": name}])
    }
    #[component]
    fn PriorityFixture() -> impl IntoElement {
        let target = binding();
        let mut model = Model::default();
        let fallback = display_name(&model, &target);
        assert!(fallback.ends_with(&fingerprint(&target)));
        assert!(!fallback.contains(&target.route) && !fallback.contains(&target.host_key));
        model.host_names =
            serde_json::from_value::<HostNames>(names(&target, "cached-host")).unwrap();
        assert_eq!(display_name(&model, &target), "cached-host");
        model.active_binding = Some(target.clone());
        model.remote_host_name = "真实主机.local".into();
        assert_eq!(display_name(&model, &target), "真实主机.local");
        model.aliases = serde_json::from_value::<HostAliases>(names(&target, "我的主机")).unwrap();
        assert_eq!(display_name(&model, &target), "我的主机");
        model.remote_host_name = "updated-server".into();
        assert_eq!(display_name(&model, &target), "我的主机");
        model.aliases = HostAliases::default();
        assert_eq!(display_name(&model, &target), "updated-server");
        model.remote_host_name.clear();
        assert_eq!(display_name(&model, &target), "cached-host");
        text("priority verified")
    }
    #[component]
    fn IsolationFixture() -> impl IntoElement {
        let target = binding();
        let mut other = target.clone();
        other.host_key = neko_remoted_protocol::encode(&[3; 32]);
        let model = Model {
            active_binding: Some(other),
            remote_host_name: "another-host".into(),
            ..Model::default()
        };
        assert_ne!(display_name(&model, &target), "another-host");
        assert!(display_name(&model, &target).ends_with(&fingerprint(&target)));
        text("isolation verified")
    }

    fn verify(root: Root, marker: &str) -> Result<(), String> {
        let mut app =
            Headless::start(EngineConfig::new(root)).map_err(|error| error.to_string())?;
        app.first_render()?;
        app.wait_for(marker)?;
        Ok(())
    }

    #[test]
    fn hostname_priority_is_alias_live_name_cache_then_decoded_fingerprint() -> Result<(), String> {
        verify(Root::of::<PriorityFixture>(), "priority verified")
    }

    #[test]
    fn hostname_from_another_selected_host_never_labels_the_current_binding() -> Result<(), String>
    {
        verify(Root::of::<IsolationFixture>(), "isolation verified")
    }
}
