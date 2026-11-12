//! Runs platform storage futures on Tokio while the NSH owner waits asynchronously.

use super::STORAGE_KEY;
use anyhow::{Context, Result};
use std::sync::OnceLock;

fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("neko-preferences")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("Cannot start preference storage worker: {error}"))
}

pub(super) async fn read() -> Result<Option<String>> {
    runtime()?
        .spawn(crate::platform::secure_get(STORAGE_KEY))
        .await
        .context("Preference storage read worker failed")?
}

pub(super) async fn write(value: String) -> Result<()> {
    runtime()?
        .spawn(async move { crate::platform::secure_set(STORAGE_KEY, &value).await })
        .await
        .context("Preference storage write worker failed")?
}
