use std::{sync::OnceLock, time::Duration};

use reqwest::{redirect::Policy, Client, StatusCode};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::external_tools::lyrica_server::LocalLyrica;

const MAX_RESPONSE_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Provider {
    Auto,
    Lrclib,
    Lrcmux,
    VideoCaptions,
    Netease,
    Megalobiz,
    Musixmatch,
    AppleMusic,
}

impl Provider {
    fn sequence(self) -> &'static str {
        match self {
            Self::Auto => "2,7,3,4,5,6,8",
            Self::Lrclib => "lrclib",
            Self::Lrcmux => "lrcmux",
            Self::VideoCaptions => crate::external_tools::SOURCE_NAME,
            Self::Netease => "netease",
            Self::Megalobiz => "megalobiz",
            Self::Musixmatch => "musixmatch",
            Self::AppleMusic => "apple_music",
        }
    }

    fn from_source(source: &str) -> Option<Self> {
        match source {
            "lrclib" => Some(Self::Lrclib),
            "lrcmux" => Some(Self::Lrcmux),
            source
                if source
                    .strip_prefix(crate::external_tools::SOURCE_NAME)
                    .is_some_and(|suffix| {
                        matches!(suffix, "" | "_music" | "_transcript" | "_subtitles")
                    }) =>
            {
                Some(Self::VideoCaptions)
            }
            "netease" => Some(Self::Netease),
            "megalobiz" => Some(Self::Megalobiz),
            "musixmatch" => Some(Self::Musixmatch),
            "apple_music" => Some(Self::AppleMusic),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LookupError {
    InvalidTrack,
    Network,
    TimedOut,
    RateLimited,
    Service,
    NotInstalled,
    LocalUnavailable,
}

#[derive(Debug, Serialize)]
pub(crate) struct Lyrics {
    provider: Option<Provider>,
    instrumental: bool,
    text: String,
    lines: Vec<TimedLine>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimedLine {
    start_ms: i64,
    text: String,
}

#[derive(Deserialize)]
struct Payload {
    status: String,
    data: Option<LyricsData>,
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

#[derive(Deserialize)]
struct LyricsData {
    source: String,
    instrumental: Option<bool>,
    lyrics: Option<String>,
    timed_lyrics: Option<Vec<ProviderLine>>,
}

#[derive(Deserialize)]
struct ProviderLine {
    start_time: Option<f64>,
    #[serde(default)]
    text: String,
}

fn decode(body: &[u8], requested: Provider) -> Result<Option<Lyrics>, LookupError> {
    let response: Payload = serde_json::from_slice(body).map_err(|_| LookupError::Service)?;
    if response.status != "success" {
        // The hosted API can return a miss in a successful HTTP response.
        return match response.error.as_ref().map(|error| error.message.as_str()) {
            Some("No lyrics found") => Ok(None),
            _ => Err(LookupError::Service),
        };
    }

    let data = response.data.ok_or(LookupError::Service)?;
    let provider = Provider::from_source(&data.source);
    // An instance may fall back to its default providers if the requested adapter is disabled.
    if provider.is_none() || (requested != Provider::Auto && provider != Some(requested)) {
        return Ok(None);
    }
    let instrumental = data.instrumental.unwrap_or(false);
    let mut lines = data
        .timed_lyrics
        .unwrap_or_default()
        .into_iter()
        .filter_map(|line| {
            let start = line.start_time?;
            (0.0..=86_400_000.0).contains(&start).then(|| TimedLine {
                start_ms: start.round() as i64,
                text: line.text,
            })
        })
        .collect::<Vec<_>>();
    lines.sort_by_key(|line| line.start_ms);
    let text = data.lyrics.unwrap_or_default();
    if !instrumental && lines.is_empty() && text.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(Lyrics {
        provider,
        instrumental,
        text,
        lines,
    }))
}

fn network_error(error: reqwest::Error) -> LookupError {
    if error.is_timeout() {
        LookupError::TimedOut
    } else {
        LookupError::Network
    }
}

#[tauri::command]
pub(crate) async fn lookup_lyrica_lyrics(
    app: AppHandle,
    server: State<'_, LocalLyrica>,
    title: String,
    artist: String,
    provider: Provider,
) -> Result<Option<Lyrics>, LookupError> {
    let title = title.trim();
    let artist = artist.trim();
    if [title, artist].into_iter().any(|value| {
        value.is_empty()
            || value.chars().take(201).count() > 200
            || value.contains(char::is_control)
    }) {
        return Err(LookupError::InvalidTrack);
    }
    let endpoint = server
        .endpoint(&app)
        .await
        .map_err(|_| LookupError::LocalUnavailable)?
        .ok_or(LookupError::NotInstalled)?;

    static CLIENT: OnceLock<Result<Client, LookupError>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(6))
                .timeout(Duration::from_secs(35))
                .build()
                .map_err(network_error)
        })
        .as_ref()
        .map_err(|_| LookupError::Network)?;
    let request = endpoint.get(client, "/lyrics/").query(&[
        ("song", title),
        ("artist", artist),
        ("timestamps", "true"),
        ("word", "false"),
        ("metadata", "false"),
        ("translate", "false"),
        ("romanize", "false"),
        ("mood", "false"),
        ("sequence", provider.sequence()),
        ("pass", "true"),
    ]);

    let mut response = request.send().await.map_err(network_error)?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::NOT_FOUND => return Ok(None),
        StatusCode::TOO_MANY_REQUESTS => return Err(LookupError::RateLimited),
        StatusCode::GATEWAY_TIMEOUT | StatusCode::REQUEST_TIMEOUT => {
            return Err(LookupError::TimedOut)
        }
        _ => return Err(LookupError::Service),
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(LookupError::Service);
        }
        body.extend_from_slice(&chunk);
    }
    decode(&body, provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_video_selection_accepts_its_subtitles_and_preserves_milliseconds() {
        let body = concat!(r#"{"status":"success","data":{"source":"you"#, r#"tube_subtitles","lyrics":"Sample",
            "timed_lyrics":[{"start_time":1850,"text":"Second"},{"start_time":-1,"text":"Invalid"},
                            {"start_time":null,"text":"No timestamp"},{"start_time":320.1,"text":"First"}]}}"#).as_bytes();
        let lyrics = decode(body, Provider::VideoCaptions).unwrap().unwrap();
        assert_eq!(lyrics.provider, Some(Provider::VideoCaptions));
        assert_eq!(
            serde_json::to_value(lyrics.lines).unwrap(),
            serde_json::json!([{"startMs":320,"text":"First"},{"startMs":1850,"text":"Second"}])
        );
        assert!(decode(body, Provider::Lrclib).unwrap().is_none());
    }

    #[test]
    fn hosted_no_match_is_distinct_from_an_api_failure() {
        let miss = br#"{"status":"error","error":{"message":"No lyrics found"}}"#;
        assert!(decode(miss, Provider::Auto).unwrap().is_none());
        let failure = br#"{"status":"error","error":{"message":"Failed to fetch lyrics"}}"#;
        assert_eq!(
            decode(failure, Provider::Auto).unwrap_err(),
            LookupError::Service
        );
    }

    #[test]
    fn rejects_genius_even_if_lyrica_returns_it_automatically() {
        let body = br#"{"status":"success","data":{"source":"genius","lyrics":"One\nTwo"}}"#;
        assert!(decode(body, Provider::Auto).unwrap().is_none());
    }

    #[test]
    fn preserves_full_lrc_alongside_partial_timed_lyrics() {
        let body = br#"{"status":"success","data":{"source":"lrclib",
            "lyrics":"[00:01.000]First\n    [00:03.000]Second",
            "timed_lyrics":[{"start_time":1000,"text":"First"}]}}"#;
        let lyrics = decode(body, Provider::Auto).unwrap().unwrap();
        assert_eq!(lyrics.text, "[00:01.000]First\n    [00:03.000]Second");
        assert_eq!(lyrics.lines.len(), 1);
    }
}
