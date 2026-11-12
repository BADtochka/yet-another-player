use std::time::Duration;

use reqwest::{redirect::Policy, Client, StatusCode, Url};
use serde::{Deserialize, Serialize};

const SEARCH_URL: &str = "https://api.genius.com/search";
const RESPONSE_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GeniusError {
    InvalidToken,
    NotConfigured,
    CredentialStore,
    Unauthorized,
    RateLimited,
    Network,
    Service,
    EmptyQuery,
}

type Result<T> = std::result::Result<T, GeniusError>;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod vault {
    use super::{GeniusError, Result};
    use keyring::{Entry, Error};

    fn entry(service: &str) -> Result<Entry> {
        Entry::new(service, "genius-client-access-token").map_err(|_| GeniusError::CredentialStore)
    }

    pub(super) fn get(service: &str) -> Result<Option<String>> {
        match entry(service)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(Error::NoEntry) => Ok(None),
            Err(_) => Err(GeniusError::CredentialStore),
        }
    }

    pub(super) fn save(service: &str, token: &str) -> Result<()> {
        entry(service)?
            .set_password(token)
            .map_err(|_| GeniusError::CredentialStore)
    }

    pub(super) fn remove(service: &str) -> Result<()> {
        match entry(service)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(_) => Err(GeniusError::CredentialStore),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod vault {
    use super::{GeniusError, Result};
    pub(super) fn get(_: &str) -> Result<Option<String>> {
        Ok(None)
    }
    pub(super) fn save(_: &str, _: &str) -> Result<()> {
        Err(GeniusError::CredentialStore)
    }
    pub(super) fn remove(_: &str) -> Result<()> {
        Err(GeniusError::CredentialStore)
    }
}

async fn credential<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| GeniusError::CredentialStore)?
}

#[tauri::command]
pub(crate) async fn genius_is_configured(app: tauri::AppHandle) -> Result<bool> {
    let service = app.config().identifier.clone();
    credential(move || Ok(vault::get(&service)?.is_some())).await
}

#[tauri::command]
pub(crate) async fn genius_save_token(app: tauri::AppHandle, token: String) -> Result<()> {
    let token = token.trim().to_owned();
    if token.is_empty() || token.len() > 2048 || !token.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(GeniusError::InvalidToken);
    }
    search(&token, "music").await?;
    let service = app.config().identifier.clone();
    credential(move || vault::save(&service, &token)).await
}

#[tauri::command]
pub(crate) async fn genius_disconnect(app: tauri::AppHandle) -> Result<()> {
    let service = app.config().identifier.clone();
    credential(move || vault::remove(&service)).await
}

#[tauri::command]
pub(crate) async fn genius_search(
    app: tauri::AppHandle,
    title: String,
    artist: String,
) -> Result<Vec<Song>> {
    if title.trim().is_empty() {
        return Err(GeniusError::EmptyQuery);
    }
    let service = app.config().identifier.clone();
    let token = credential(move || vault::get(&service))
        .await?
        .ok_or(GeniusError::NotConfigured)?;
    let artist: String = artist.trim().chars().take(160).collect();
    let title: String = title.trim().chars().take(160).collect();
    let query = format!("{artist} {title}");
    search(&token, query.trim()).await
}

#[derive(Serialize)]
pub(crate) struct Song {
    title: String,
    artist: String,
    url: String,
}

#[derive(Deserialize)]
struct SearchEnvelope {
    response: SearchResults,
}

#[derive(Deserialize)]
struct SearchResults {
    hits: Vec<SearchHit>,
}

#[derive(Deserialize)]
struct SearchHit {
    #[serde(rename = "type")]
    kind: String,
    result: serde_json::Value,
}

#[derive(Deserialize)]
struct SongResult {
    title: String,
    artist_names: String,
    url: String,
}

fn parse_results(data: &[u8]) -> Result<Vec<Song>> {
    let results: SearchEnvelope = serde_json::from_slice(data).map_err(|_| GeniusError::Service)?;
    Ok(results
        .response
        .hits
        .into_iter()
        .filter(|hit| hit.kind == "song")
        .filter_map(|hit| serde_json::from_value::<SongResult>(hit.result).ok())
        .filter_map(|song| {
            let url = Url::parse(&song.url).ok()?;
            if url.scheme() != "https"
                || url.host_str() != Some("genius.com")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || song.title.trim().is_empty()
            {
                return None;
            }
            Some(Song {
                title: song.title,
                artist: song.artist_names,
                url: url.into(),
            })
        })
        .take(5)
        .collect())
}

async fn search(token: &str, query: &str) -> Result<Vec<Song>> {
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| GeniusError::Network)?;
    let mut response = client
        .get(SEARCH_URL)
        .bearer_auth(token)
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|_| GeniusError::Network)?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Err(GeniusError::Unauthorized),
        StatusCode::TOO_MANY_REQUESTS => return Err(GeniusError::RateLimited),
        _ => return Err(GeniusError::Service),
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| GeniusError::Network)? {
        if data.len() + chunk.len() > RESPONSE_LIMIT {
            return Err(GeniusError::Service);
        }
        data.extend_from_slice(&chunk);
    }
    parse_results(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_only_returns_songs_with_trusted_links() {
        let data = br#"{"response":{"hits":[
            {"type":"song","result":{"title":"Song","artist_names":"Artist","url":"https://genius.com/Artist-song-lyrics"}},
            {"type":"song","result":{"title":"Fake","artist_names":"Artist","url":"https://genius.com.example.org/fake"}},
            {"type":"song","result":{"title":"Fake","artist_names":"Artist","url":"https://fake@genius.com/fake"}},
            {"type":"artist","result":{"title":"Artist","artist_names":"Artist","url":"https://genius.com/artists/Artist"}},
            {"type":"song","result":{"title":"Incomplete"}}
        ]}}"#;
        let results = parse_results(data).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Song");
        assert_eq!(results[0].artist, "Artist");
        assert_eq!(results[0].url, "https://genius.com/Artist-song-lyrics");
    }
}
