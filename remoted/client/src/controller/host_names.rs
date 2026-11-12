use super::*;

impl Actor {
    pub(super) async fn reported_host_name(&mut self, name: String) {
        let Some(binding) = self.binding.clone().filter(|binding| {
            self.model.ready()
                && !self.model.local
                && self.model.active_binding.as_ref() == Some(binding)
                && self.model.bindings.contains(binding)
                && self.model.generation == self.scope.current()
        }) else {
            return;
        };
        let Ok(name) = storage::normalize_host_name(&name) else {
            return;
        };
        if self.model.host_names.get(&binding) == Some(name.as_str()) {
            self.model.remote_host_name = name;
            return;
        }
        self.reported_name_with(
            &binding,
            &name,
            storage::remember_host_name(&binding, &name),
        )
        .await;
    }

    async fn reported_name_with(
        &mut self,
        binding: &Binding,
        name: &str,
        persist: impl std::future::Future<Output = Result<storage::HostNames>>,
    ) {
        if !self.model.bindings.contains(binding)
            || self.model.active_binding.as_ref() != Some(binding)
            || self.model.generation != self.scope.current()
        {
            return;
        }
        // The authenticated live hostname is useful even if caching is unavailable.
        // Only a confirmed write updates the durable-name view used while offline.
        self.model.remote_host_name = name.to_owned();
        match persist.await {
            Ok(mut names) => {
                names.retain(&self.model.bindings);
                self.model.host_names = names;
            }
            Err(_) => {
                self.model.notice = "远程主机名未能确认保存，下次连接时会重试。".into();
            }
        }
    }

    pub(super) async fn rename_binding(&mut self, binding: Binding, alias: String) {
        if let Err(error) = storage::normalize_host_alias(&alias) {
            self.model.error = error.to_string();
            return;
        }
        self.rename_with(&binding, storage::rename_binding(&binding, &alias))
            .await;
    }

    async fn rename_with(
        &mut self,
        binding: &Binding,
        persist: impl std::future::Future<Output = Result<storage::HostAliases>>,
    ) {
        self.model.error.clear();
        if !self.model.bindings.contains(binding) {
            self.model.error = "该设备配对已变化，请重新选择".into();
            return;
        }
        self.model.alias_saving = Some(binding.clone());
        self.publish().await;
        match persist.await {
            Ok(mut aliases) => {
                aliases.retain(&self.model.bindings);
                self.model.aliases = aliases;
            }
            Err(_) => {
                self.model.error = "主机别名未能确认保存，请重试；仍显示上次确认的名称。".into();
            }
        }
        self.model.alias_saving = None;
    }
}

#[cfg(test)]
mod tests;
