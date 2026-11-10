//! Pinned Tencent model download, checked extraction and atomic installation.
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

pub(super) const URL: &str = "https://download.z.weixin.qq.com/publish_words/v1/ime_asr_decoder_model/20260710220526250_ime_asr_decoder_model.apk";
pub(super) const SIZE: usize = 102_920_639;
pub(super) const SHA256: &str = "b3729fbe928b25740cc9cd781537175b1db4a445bd78bf464267c0c4633e0863";
const INNER: &str = "ime_asr_decoder_model.zip";
const MODEL: &str = "embed_140m_kld43wh_test_78000_1_merge_4.kv.conv1d.dyQu.onnx.xnet";
const DICT: &str = "dict.decoder.utf8.txt";
const MODEL_SHA256: &str = "0674250a848ce3eb91024b3c0a6eda61b91ea45b2c40e4900130995e89cb9033";
const DICT_SHA256: &str = "08a16004cfcf5b9f1668e055f3100de9b8a44f88541819def2bbd56bfd42f484";

pub(super) fn ready(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("READY")).is_ok_and(|value| value == SHA256)
        && std::fs::metadata(dir.join(MODEL)).is_ok_and(|m| m.is_file() && m.len() == 135_646_496)
        && std::fs::metadata(dir.join(DICT)).is_ok_and(|m| m.is_file() && m.len() == 129_706)
}
pub(super) async fn download() -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_mins(3))
        .build()?;
    let response = client.get(URL).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size != SIZE as u64)
    {
        bail!("unexpected model package length");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::with_capacity(SIZE);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.len() > SIZE.saturating_sub(bytes.len()) {
            bail!("model package exceeds its pinned size");
        }
        bytes.extend_from_slice(&chunk);
    }
    verify(&bytes, SIZE, SHA256)?;
    Ok(bytes)
}
fn verify(bytes: &[u8], size: usize, expected: &str) -> Result<()> {
    let digest = Sha256::digest(bytes);
    let matches_hash = expected.len() == digest.len() * 2
        && digest
            .iter()
            .zip(expected.as_bytes().chunks_exact(2))
            .all(|(byte, pair)| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                pair == [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)]]
            });
    if bytes.len() != size || !matches_hash {
        bail!("model integrity check failed");
    }
    Ok(())
}
pub(super) fn verify_installed(dir: &Path) -> Result<()> {
    if !ready(dir) {
        bail!("download the WeChat speech model in plugin settings first");
    }
    verify(&std::fs::read(dir.join(MODEL))?, 135_646_496, MODEL_SHA256)?;
    verify(&std::fs::read(dir.join(DICT))?, 129_706, DICT_SHA256)
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn install(bytes: Vec<u8>, dir: &Path) -> Result<()> {
    verify(&bytes, SIZE, SHA256)?;
    let parent = dir.parent().context("invalid model directory")?;
    std::fs::create_dir_all(parent)?;
    let staging = Staging(parent.join(format!(".wechat-asr-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&staging.0)?;
    let mut outer = ZipArchive::new(Cursor::new(bytes))?;
    let inner = read_entry(&mut outer, INNER, 102_913_462)?;
    let mut inner = ZipArchive::new(Cursor::new(inner))?;
    for (name, size, hash) in [
        (MODEL, 135_646_496, MODEL_SHA256),
        (DICT, 129_706, DICT_SHA256),
    ] {
        let contents = read_entry(&mut inner, name, size)?;
        verify(&contents, size, hash)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.0.join(name))?;
        file.write_all(&contents)?;
        file.sync_all()?;
    }
    // Parse only the authenticated, pinned model bytes before making it visible.
    drop(wowtype_voice::wtlocal::Model::load(&staging.0)?);
    std::fs::write(staging.0.join("READY"), SHA256)?;
    if dir.exists() {
        let backup = parent.join(format!(".wechat-asr-old-{}", uuid::Uuid::new_v4()));
        std::fs::rename(dir, &backup)?;
        if let Err(error) = std::fs::rename(&staging.0, dir) {
            // Keep the backup if restoration fails, so the last valid model is recoverable.
            std::fs::rename(&backup, dir).context("restore prior model directory")?;
            return Err(error.into());
        }
        let _ = std::fs::remove_dir_all(backup);
    } else {
        std::fs::rename(&staging.0, dir)?;
    }
    Ok(())
}
fn read_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    size: usize,
) -> Result<Vec<u8>> {
    let file = archive.by_name(name)?;
    if file.size() != size as u64 || file.is_symlink() {
        bail!("unexpected model archive entry");
    }
    let mut bytes = Vec::with_capacity(size);
    file.take(size as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() != size {
        bail!("invalid model archive entry size");
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    #[test]
    fn invalid_download_is_never_installed() {
        let dir = std::env::temp_dir().join(format!("neko-asr-invalid-{}", uuid::Uuid::new_v4()));
        assert!(super::install(vec![0; 128], &dir).is_err());
        assert!(!dir.exists());
        assert!(!super::ready(&dir));
    }
    #[test]
    fn malformed_same_size_download_fails_integrity() {
        assert!(super::verify(b"abc", 3, "bad hash").is_err());
    }
    #[test]
    #[ignore = "explicit model download smoke test; network and approximately 240 MiB disk required"]
    fn installed_real_model_loads_and_transcribes_silence() {
        let package = std::env::var_os("NEKO_ASR_TEST_PACKAGE").expect("test model package path");
        let dir = std::env::temp_dir().join(format!("neko-asr-real-{}", uuid::Uuid::new_v4()));
        let cleanup = super::Staging(dir.clone());
        super::install(std::fs::read(package).unwrap(), &dir).unwrap();
        super::verify_installed(&dir).unwrap();
        let model = std::sync::Arc::new(wowtype_voice::wtlocal::Model::load(&dir).unwrap());
        let mut stream = wowtype_voice::wtlocal::Stream::new(model);
        stream.push(&vec![0; 16_000]);
        assert!(stream.finish().trim().is_empty());
        drop(cleanup);
    }
    #[test]
    #[ignore = "requires the pinned model package and a generated Chinese speech fixture"]
    fn installed_real_model_recognizes_speech() {
        let package = std::env::var_os("NEKO_ASR_TEST_PACKAGE").expect("test model package path");
        let audio = std::env::var_os("NEKO_ASR_TEST_WAV").expect("test Chinese PCM WAV path");
        let dir = std::env::temp_dir().join(format!("neko-asr-speech-{}", uuid::Uuid::new_v4()));
        let _cleanup = super::Staging(dir.clone());
        super::install(std::fs::read(package).unwrap(), &dir).unwrap();
        let model = std::sync::Arc::new(wowtype_voice::wtlocal::Model::load(&dir).unwrap());
        let mut stream = wowtype_voice::wtlocal::Stream::new(model);
        stream.push(&super::super::audio::pcm(&std::fs::read(audio).unwrap()).unwrap());
        let text = stream.finish();
        assert!(
            text.contains("语音识别") && text.contains("测试"),
            "recognized: {text}"
        );
        eprintln!("recognized generated test speech: {text}");
    }
}
