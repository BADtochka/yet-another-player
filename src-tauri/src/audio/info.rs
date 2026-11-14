use std::{panic::catch_unwind, path::Path, sync::Arc};

use lofty::{
    config::ParseOptions,
    file::{AudioFile, FileType, TaggedFileExt},
    probe::Probe,
};
use serde::Serialize;
use symphonia::core::codecs::{
    CODEC_TYPE_AAC, CODEC_TYPE_ALAC, CODEC_TYPE_FLAC, CODEC_TYPE_MP3, CODEC_TYPE_OPUS,
    CODEC_TYPE_VORBIS,
};
use tauri::State;

use crate::MusicFolders;

use super::{library_track_path, source};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioInfo {
    format: String,
    codec: Option<&'static str>,
    bitrate_kbps: Option<u32>,
    sample_rate_hz: Option<u32>,
    bit_depth: Option<u32>,
}

#[tauri::command]
pub(crate) async fn track_audio_info(
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<AudioInfo, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let path = library_track_path(&path, &roots)?;
        Ok(read(&path))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn read(path: &Path) -> AudioInfo {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let tagged = catch_unwind(|| {
        Probe::open(path)
            .ok()?
            .guess_file_type()
            .ok()?
            .options(ParseOptions::new().read_tags(false))
            .read()
            .ok()
    })
    .ok()
    .flatten();
    let kind = tagged.as_ref().map(TaggedFileExt::file_type);
    let format = match kind {
        Some(FileType::Flac) => "FLAC".into(),
        Some(FileType::Mpeg) => "MP3".into(),
        Some(FileType::Mp4) => "M4A".into(),
        Some(FileType::Wav) => "WAV".into(),
        Some(FileType::Vorbis) => "OGG".into(),
        Some(FileType::Opus) if extension.eq_ignore_ascii_case("ogg") => "OGG".into(),
        Some(FileType::Opus) => "Opus".into(),
        _ if extension.eq_ignore_ascii_case("opus") => "Opus".into(),
        _ if extension.eq_ignore_ascii_case("webm") => "WebM".into(),
        _ => extension.to_ascii_uppercase(),
    };
    let properties = tagged.as_ref().map(AudioFile::properties);
    let codec = match kind {
        Some(FileType::Vorbis) => Some("Vorbis"),
        Some(FileType::Opus) => Some("Opus"),
        _ => None,
    };
    let mut info = AudioInfo {
        format,
        codec,
        bitrate_kbps: properties
            .and_then(|value| value.audio_bitrate())
            .filter(|value| *value > 0),
        sample_rate_hz: properties
            .and_then(|value| value.sample_rate())
            .filter(|value| *value > 0),
        bit_depth: properties
            .and_then(|value| value.bit_depth())
            .filter(|value| *value > 0)
            .map(u32::from),
    };
    if kind == Some(FileType::Mp4) || info.sample_rate_hz.is_none() {
        if let Some(parameters) = catch_unwind(|| {
            let reader = source::probe_format(path).ok()?;
            reader
                .default_track()
                .map(|track| track.codec_params.clone())
        })
        .ok()
        .flatten()
        {
            info.codec = match parameters.codec {
                CODEC_TYPE_AAC => Some("AAC"),
                CODEC_TYPE_ALAC => Some("ALAC"),
                CODEC_TYPE_FLAC => Some("FLAC"),
                CODEC_TYPE_MP3 => Some("MP3"),
                CODEC_TYPE_OPUS => Some("Opus"),
                CODEC_TYPE_VORBIS => Some("Vorbis"),
                _ => info.codec,
            };
            info.sample_rate_hz = info
                .sample_rate_hz
                .or(parameters.sample_rate.filter(|rate| *rate > 0));
            info.bit_depth = info
                .bit_depth
                .or(parameters.bits_per_sample.filter(|depth| *depth > 0));
        }
    }
    if info.codec == Some("Opus") {
        info.sample_rate_hz = Some(48_000);
    }
    if kind == Some(FileType::Mpeg) || matches!(info.codec, Some("AAC" | "MP3" | "Opus" | "Vorbis"))
    {
        info.bit_depth = None;
    }
    if info
        .codec
        .is_some_and(|codec| codec.eq_ignore_ascii_case(&info.format))
    {
        info.codec = None;
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pcm_quality_and_recognizes_opus_without_lofty_properties() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sample.wav");
        std::fs::write(&wav, b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\x02\0\0\0\0\0").unwrap();
        let info = read(&wav);
        assert_eq!(info.format, "WAV");
        assert_eq!(info.sample_rate_hz, Some(8_000));
        assert_eq!(info.bit_depth, Some(16));

        let opus = dir.path().join("sample.ogg");
        std::fs::write(&opus, include_bytes!("../library/fixtures/one-sample.opus")).unwrap();
        let info = read(&opus);
        assert_eq!(info.format, "OGG");
        assert_eq!(info.codec, Some("Opus"));
        assert_eq!(info.sample_rate_hz, Some(48_000));
        assert_eq!(info.bit_depth, None);
    }
}
