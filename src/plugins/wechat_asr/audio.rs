//! Bounded browser WAV input; the inference engine receives 16 kHz mono PCM.
use neko_plugin::PluginError;
use wowtype_voice::audio::Resampler;

pub(super) const MAX_SECONDS: usize = 60;
const MAX_UPLOAD: usize = 25 * 1024 * 1024;

pub(super) fn pcm(bytes: &[u8]) -> Result<Vec<i16>, PluginError> {
    if bytes.len() > MAX_UPLOAD {
        return Err(PluginError::resource_exhausted(
            "voice recording exceeds 25 MiB",
        ));
    }
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(PluginError::invalid_argument(
            "local ASR requires a PCM WAV recording",
        ));
    }
    let declared = usize::try_from(u32_at(bytes, 4)?).map_err(invalid)?;
    if declared.checked_add(8) != Some(bytes.len()) {
        return Err(invalid("invalid WAV length"));
    }
    let mut offset = 12_usize;
    let mut format = None;
    let mut audio = None;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or_else(|| invalid("truncated WAV chunk"))?;
        let size = usize::try_from(u32_at(header, 4)?).map_err(invalid)?;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .ok_or_else(|| invalid("invalid WAV chunk size"))?;
        let chunk = bytes
            .get(start..end)
            .ok_or_else(|| invalid("truncated WAV chunk"))?;
        match &header[..4] {
            b"fmt " => {
                if format.is_some() || chunk.len() < 16 {
                    return Err(invalid("invalid WAV format"));
                }
                let channels = u16_at(chunk, 2)?;
                let rate = u32_at(chunk, 4)?;
                if u16_at(chunk, 0)? != 1
                    || u16_at(chunk, 14)? != 16
                    || !(1..=2).contains(&channels)
                    || !(8_000..=96_000).contains(&rate)
                    || u16_at(chunk, 12)? != channels * 2
                    || u32_at(chunk, 8)? != rate * u32::from(channels) * 2
                {
                    return Err(invalid(
                        "local ASR requires 16-bit mono/stereo PCM WAV at 8–96 kHz",
                    ));
                }
                format = Some((usize::from(channels), rate));
            }
            b"data" => {
                if audio.is_some() {
                    return Err(invalid("duplicate WAV audio"));
                }
                audio = Some(chunk);
            }
            _ => {}
        }
        offset = end
            .checked_add(size % 2)
            .ok_or_else(|| invalid("invalid WAV padding"))?;
        if offset > bytes.len() {
            return Err(invalid("missing WAV padding"));
        }
    }
    let (channels, rate) = format.ok_or_else(|| invalid("missing WAV format"))?;
    let audio = audio.ok_or_else(|| invalid("missing WAV audio"))?;
    let frame_size = channels * 2;
    if audio.is_empty() || audio.len() % frame_size != 0 {
        return Err(invalid("empty or incomplete WAV audio"));
    }
    let frames = audio.len() / frame_size;
    if frames > usize::try_from(rate).map_err(invalid)? * MAX_SECONDS {
        return Err(PluginError::resource_exhausted(
            "local ASR recordings are limited to 60 seconds",
        ));
    }
    let mono: Vec<f32> = audio
        .chunks_exact(frame_size)
        .map(|frame| {
            let left = f32::from(i16::from_le_bytes([frame[0], frame[1]]));
            if channels == 1 {
                left
            } else {
                left.midpoint(f32::from(i16::from_le_bytes([frame[2], frame[3]])))
            }
        })
        .collect();
    #[allow(clippy::cast_possible_truncation)]
    // Averaging/filtering signed PCM stays within i16; float cast saturates.
    Ok(Resampler::new(rate, 16_000)
        .process(&mono)
        .into_iter()
        .map(|value| value.round() as i16)
        .collect())
}
fn invalid(message: impl std::fmt::Display) -> PluginError {
    PluginError::invalid_argument(message.to_string())
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, PluginError> {
    let b = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("truncated WAV header"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, PluginError> {
    let b = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("truncated WAV header"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::pcm;
    fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let size = u32::try_from(samples.len() * 2).unwrap();
        let mut out = b"RIFF".to_vec();
        out.extend((size + 36).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16_u32.to_le_bytes());
        out.extend(1_u16.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(channels) * 2).to_le_bytes());
        out.extend((channels * 2).to_le_bytes());
        out.extend(16_u16.to_le_bytes());
        out.extend(b"data");
        out.extend(size.to_le_bytes());
        for s in samples {
            out.extend(s.to_le_bytes());
        }
        out
    }
    #[test]
    fn native_mono_round_trip() {
        assert_eq!(
            pcm(&wav(16_000, 1, &[123, -456, 789])).unwrap(),
            [123, -456, 789]
        );
    }
    #[test]
    fn browser_stereo_is_resampled() {
        let got = pcm(&wav(48_000, 2, &vec![100; 96_000])).unwrap();
        assert!((15_990..=16_001).contains(&got.len()));
        assert!(got.iter().skip(50).all(|s| (95..=105).contains(s)));
    }
    #[test]
    fn rejects_truncation_compression_and_excess_duration() {
        let mut data = wav(16_000, 1, &[1, 2]);
        assert!(pcm(&data[..data.len() - 1]).is_err());
        data[20] = 3;
        assert!(pcm(&data).is_err());
        assert!(pcm(&wav(8_000, 1, &vec![0; 8_000 * 61])).is_err());
        assert!(pcm(&wav(16_000, 1, &[])).is_err());
    }
    #[test]
    fn all_truncated_headers_are_errors() {
        let data = wav(16_000, 1, &[1, 2]);
        for end in 0..data.len() {
            assert!(pcm(&data[..end]).is_err(), "length {end}");
        }
    }
}
