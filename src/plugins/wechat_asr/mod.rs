mod audio;
mod model;

use neko_plugin::{
    AsrModelDirectory, BoxFuture, InvokeReply, InvokeRequest, PluginConfiguration, PluginError,
    Resources, RuntimePlugin,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use wowtype_voice::wtlocal::{Model, Stream};

pub fn register(resources: &Resources) -> Result<Arc<dyn RuntimePlugin>, PluginError> {
    let directory = resources.require::<AsrModelDirectory>("asr-models")?;
    Ok(Arc::new(WeChatAsr {
        directory,
        operation: Mutex::new(None),
    }))
}
struct WeChatAsr {
    directory: Arc<AsrModelDirectory>,
    operation: Mutex<Option<Arc<Model>>>,
}
fn failed(error: impl std::fmt::Display) -> PluginError {
    PluginError::unavailable(error.to_string())
}
impl WeChatAsr {
    fn status(&self) -> Result<InvokeReply, PluginError> {
        InvokeReply::json(
            "ok",
            &serde_json::json!({"ready": model::ready(&self.directory.path), "downloadBytes": model::SIZE, "maxSeconds":audio::MAX_SECONDS,"offline":true}),
        )
    }
}
impl RuntimePlugin for WeChatAsr {
    fn id(&self) -> &'static str {
        "wechat-asr"
    }
    fn invoke(&self, request: InvokeRequest) -> BoxFuture<'_, Result<InvokeReply, PluginError>> {
        Box::pin(async move {
            if request.operation == "status" {
                return self.status();
            }
            if !matches!(
                request.operation.as_str(),
                "download" | "remove" | "transcribe"
            ) {
                return Err(PluginError::invalid_argument("unknown ASR operation"));
            }
            // Bound queueing: an overlapping call must not retain another large audio buffer.
            let mut engine = self.operation.try_lock().map_err(|_| {
                PluginError::resource_exhausted(
                    "speech provider is busy; try again after the current operation",
                )
            })?;
            let dir = self.directory.path.clone();
            match request.operation.as_str() {
                "download" => {
                    if !model::ready(&dir) {
                        let bytes = model::download().await.map_err(failed)?;
                        tokio::task::spawn_blocking(move || model::install(bytes, &dir))
                            .await
                            .map_err(failed)?
                            .map_err(failed)?;
                    }
                    self.status()
                }
                "remove" => {
                    *engine = None;
                    tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(dir) {
                        Ok(()) => Ok(()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(error),
                    })
                    .await
                    .map_err(failed)?
                    .map_err(failed)?;
                    self.status()
                }
                "transcribe" => {
                    if !matches!(
                        request
                            .content_type
                            .split(';')
                            .next()
                            .unwrap_or_default()
                            .trim(),
                        "audio/wav" | "audio/wave" | "audio/x-wav"
                    ) {
                        return Err(PluginError::invalid_argument(
                            "local ASR requires WAV recording format",
                        ));
                    }
                    let cached = engine.clone();
                    let (loaded, text) = tokio::task::spawn_blocking(move || {
                        let pcm = audio::pcm(&request.payload)?;
                        let loaded = if let Some(model) = cached {
                            model
                        } else {
                            model::verify_installed(&dir).map_err(|error| {
                                PluginError::failed_precondition(error.to_string())
                            })?;
                            Arc::new(Model::load(&dir).map_err(failed)?)
                        };
                        let mut stream = Stream::new(Arc::clone(&loaded));
                        stream.push(&pcm);
                        Ok::<_, PluginError>((loaded, stream.finish()))
                    })
                    .await
                    .map_err(failed)??;
                    *engine = Some(loaded);
                    InvokeReply::json("ok", &serde_json::json!({"text":text.trim()}))
                }
                _ => Err(PluginError::invalid_argument("unknown ASR operation")),
            }
        })
    }
    fn configure(
        &self,
        configuration: PluginConfiguration,
    ) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            if !configuration.enabled {
                *self.operation.lock().await = None;
            }
            Ok(())
        })
    }
    fn shutdown(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            *self.operation.lock().await = None;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> WeChatAsr {
        WeChatAsr {
            directory: Arc::new(AsrModelDirectory {
                path: std::env::temp_dir()
                    .join(format!("neko-asr-contract-{}", uuid::Uuid::new_v4())),
            }),
            operation: Mutex::new(None),
        }
    }
    fn request(operation: &str) -> InvokeRequest {
        InvokeRequest {
            target: neko_plugin::TargetRequest::default(),
            operation: operation.into(),
            content_type: "audio/wav".into(),
            payload: Vec::new(),
        }
    }

    #[tokio::test]
    async fn overlapping_audio_is_rejected_and_status_remains_available() {
        let provider = provider();
        let _gate = provider.operation.lock().await;
        assert_eq!(
            provider
                .invoke(request("transcribe"))
                .await
                .unwrap_err()
                .code,
            neko_plugin::ErrorCode::ResourceExhausted
        );
        let status = provider.invoke(request("status")).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&status.payload).unwrap()["ready"],
            false
        );
    }

    #[tokio::test]
    async fn removal_is_idempotent_and_invalid_audio_does_not_initialize_a_model() {
        let provider = provider();
        std::fs::create_dir_all(&provider.directory.path).unwrap();
        std::fs::write(provider.directory.path.join("partial"), b"partial").unwrap();
        assert_eq!(
            provider
                .invoke(request("transcribe"))
                .await
                .unwrap_err()
                .code,
            neko_plugin::ErrorCode::InvalidArgument
        );
        assert!(provider.operation.lock().await.is_none());
        provider.invoke(request("remove")).await.unwrap();
        provider.invoke(request("remove")).await.unwrap();
        assert!(!provider.directory.path.exists());
    }
}
