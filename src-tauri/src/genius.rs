use std::time::Duration;

use dom_query::Document;
use reqwest::{redirect::Policy, Client, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};

const SEARCH_URL: &str = "https://api.genius.com/search";
const LYRICS_PAGE_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 ",
    "(KHTML, like Gecko) Version/18.6 Safari/605.1.15"
);
const RESPONSE_LIMIT: usize = 1024 * 1024;
const PAGE_LIMIT: usize = 4 * 1024 * 1024;
const TEXT_LIMIT: usize = 256 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GeniusError {
    InvalidToken,
    NotConfigured,
    Disabled,
    CredentialStore,
    Unauthorized,
    RateLimited,
    Network,
    Service,
    EmptyQuery,
    PageUnavailable { url: String },
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
pub(crate) async fn genius_lyrics(
    app: tauri::AppHandle,
    title: String,
    artists: Vec<String>,
) -> Result<Option<String>> {
    if !tool_enabled(&app) {
        return Err(GeniusError::Disabled);
    }
    let title: String = title.trim().chars().take(160).collect();
    let artists = artists
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .take(12)
        .map(|name| name.chars().take(160).collect::<String>())
        .collect::<Vec<_>>();
    if title.is_empty() || artists.is_empty() {
        return Err(GeniusError::EmptyQuery);
    }
    let token = access_token(&app)
        .await?
        .ok_or(GeniusError::NotConfigured)?;
    let Some(song) = find_song(&token, &title, &artists, |song| !song.instrumental).await? else {
        return Ok(None);
    };
    if !tool_enabled(&app) {
        return Err(GeniusError::Disabled);
    }
    fetch_lyrics_page(song.url).await
}

pub(crate) fn tool_enabled(app: &tauri::AppHandle) -> bool {
    crate::external_tools::enabled::tool_enabled(app, crate::external_tools::install::Tool::Genius)
}

pub(crate) async fn access_token(app: &tauri::AppHandle) -> Result<Option<String>> {
    let service = app.config().identifier.clone();
    credential(move || vault::get(&service)).await
}

#[derive(Deserialize)]
pub(crate) struct MetadataAlbum {
    pub(crate) name: String,
    pub(crate) artist: Option<ArtistCredit>,
}

#[derive(Deserialize)]
pub(crate) struct MetadataSong {
    pub(crate) title: String,
    pub(crate) artist_names: String,
    pub(crate) album: Option<MetadataAlbum>,
    pub(crate) release_date: Option<String>,
}

#[derive(Deserialize)]
struct MetadataResponse {
    response: MetadataSongResponse,
}

#[derive(Deserialize)]
struct MetadataSongResponse {
    song: MetadataSong,
}

pub(crate) async fn metadata(
    token: &str,
    title: &str,
    artists: &[String],
) -> Result<Option<MetadataSong>> {
    let Some(id) = find_song(token, title, artists, |song| song.id.is_some())
        .await?
        .and_then(|song| song.id)
    else {
        return Ok(None);
    };
    let response = client()?
        .get(format!("https://api.genius.com/songs/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| GeniusError::Network)?;
    match response.status() {
        StatusCode::OK => {
            let bytes = read_bounded(response, RESPONSE_LIMIT).await?;
            let result: MetadataResponse =
                serde_json::from_slice(&bytes).map_err(|_| GeniusError::Service)?;
            Ok(Some(result.response.song))
        }
        StatusCode::NOT_FOUND => Ok(None),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(GeniusError::Unauthorized),
        StatusCode::TOO_MANY_REQUESTS => Err(GeniusError::RateLimited),
        _ => Err(GeniusError::Service),
    }
}

pub(crate) async fn artist_avatar(
    token: &str,
    title: &str,
    artist: &str,
) -> Result<Option<String>> {
    let song = find_song(token, title, &[artist.to_owned()], |song| {
        song.primary_artist
            .as_deref()
            .is_some_and(|name| normalize(name) == normalize(artist))
    })
    .await?;
    Ok(song.and_then(|song| song.artist_image))
}

struct Song {
    id: Option<u64>,
    instrumental: bool,
    title: String,
    artist: String,
    primary_artist: Option<String>,
    artist_image: Option<String>,
    url: Url,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SongMatch {
    TranslatedTitle,
    ExactTitle,
    ExactTitleAndArtists,
}

impl Song {
    fn match_rank(&self, title: &str, artists: &[String]) -> Option<SongMatch> {
        let primary = normalize(artists.first()?);
        let credited = normalize(&self.artist);
        let full_credit = normalize(&artists.join(" ")) == credited;
        let primary_credit = credited == primary
            || self
                .primary_artist
                .as_deref()
                .is_some_and(|name| normalize(name) == primary);
        let normalized_title = normalize(title);
        if primary.is_empty() || normalized_title.is_empty() || (!full_credit && !primary_credit) {
            return None;
        }
        if normalize(&self.title) == normalized_title {
            return Some(if full_credit {
                SongMatch::ExactTitleAndArtists
            } else {
                SongMatch::ExactTitle
            });
        }
        if (full_credit || artists.len() == 1)
            && (has_translated_title(&self.title, title)
                || has_translated_title(title, &self.title))
        {
            return Some(SongMatch::TranslatedTitle);
        }
        None
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .map(|ch| if ch == 'ё' { 'е' } else { ch })
        .collect()
}

fn has_translated_title(expanded: &str, short: &str) -> bool {
    let Some((original, alternate)) = expanded
        .trim()
        .strip_suffix(')')
        .and_then(|title| title.rsplit_once(" ("))
    else {
        return false;
    };
    original
        .chars()
        .any(|ch| matches!(ch, '\u{0400}'..='\u{052f}'))
        && normalize(original) == normalize(short)
        && alternate.chars().any(|ch| ch.is_ascii_alphabetic())
        && !alternate
            .chars()
            .any(|ch| ch.is_alphabetic() && !ch.is_ascii_alphabetic())
        && !alternate
            .split(|ch: char| !ch.is_ascii_alphabetic())
            .any(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "acoustic"
                        | "alternate"
                        | "cover"
                        | "demo"
                        | "edit"
                        | "extended"
                        | "instrumental"
                        | "karaoke"
                        | "live"
                        | "mix"
                        | "radio"
                        | "remaster"
                        | "remastered"
                        | "remix"
                        | "rendition"
                        | "slowed"
                        | "sped"
                        | "version"
                )
            })
}

fn best_song(
    songs: Vec<Song>,
    title: &str,
    artists: &[String],
    eligible: &impl Fn(&Song) -> bool,
) -> Option<Song> {
    songs
        .into_iter()
        .rev()
        .filter(eligible)
        .filter_map(|song| song.match_rank(title, artists).map(|rank| (rank, song)))
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, song)| song)
}

async fn find_song(
    token: &str,
    title: &str,
    artists: &[String],
    eligible: impl Fn(&Song) -> bool,
) -> Result<Option<Song>> {
    let Some(artist) = artists.first() else {
        return Ok(None);
    };
    if let Some(song) = best_song(
        search(token, &format!("{artist} {title}")).await?,
        title,
        artists,
        &eligible,
    ) {
        return Ok(Some(song));
    }
    Ok(best_song(
        search(token, title).await?,
        title,
        artists,
        &eligible,
    ))
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
    id: Option<u64>,
    title: String,
    artist_names: String,
    url: String,
    primary_artist: Option<ArtistCredit>,
    instrumental: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct ArtistCredit {
    pub(crate) name: String,
    image_url: Option<String>,
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
                || !url.path().ends_with("-lyrics")
                || song.title.trim().is_empty()
            {
                return None;
            }
            Some(Song {
                id: song.id,
                instrumental: song.instrumental.unwrap_or(false),
                title: song.title,
                artist: song.artist_names,
                artist_image: song
                    .primary_artist
                    .as_ref()
                    .and_then(|artist| artist.image_url.clone()),
                primary_artist: song.primary_artist.map(|artist| artist.name),
                url,
            })
        })
        .take(5)
        .collect())
}

async fn search(token: &str, query: &str) -> Result<Vec<Song>> {
    let response = client()?
        .get(SEARCH_URL)
        .bearer_auth(token)
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|_| GeniusError::Network)?;
    match response.status() {
        StatusCode::OK => parse_results(&read_bounded(response, RESPONSE_LIMIT).await?),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(GeniusError::Unauthorized),
        StatusCode::TOO_MANY_REQUESTS => Err(GeniusError::RateLimited),
        _ => Err(GeniusError::Service),
    }
}

fn client() -> Result<Client> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| GeniusError::Network)
}

async fn fetch_lyrics_page(url: Url) -> Result<Option<String>> {
    // Search returns the public song URL. The credential is scoped to the search API.
    let response = client()?
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "text/html")
        .header(reqwest::header::USER_AGENT, LYRICS_PAGE_USER_AGENT)
        .send()
        .await
        .map_err(|_| GeniusError::Network)?;
    if !lyrics_page_available(response.status(), url)? {
        return Ok(None);
    }
    let page = read_bounded(response, PAGE_LIMIT).await?;
    let page = String::from_utf8(page).map_err(|_| GeniusError::Service)?;
    tauri::async_runtime::spawn_blocking(move || extract_lyrics(&page))
        .await
        .map_err(|_| GeniusError::Service)
}

fn lyrics_page_available(status: StatusCode, url: Url) -> Result<bool> {
    match status {
        StatusCode::OK => Ok(true),
        StatusCode::NOT_FOUND => Ok(false),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(GeniusError::PageUnavailable { url: url.into() })
        }
        StatusCode::TOO_MANY_REQUESTS => Err(GeniusError::RateLimited),
        _ => Err(GeniusError::Service),
    }
}

fn extract_lyrics(page: &str) -> Option<String> {
    let document = Document::from(page);
    let sections = document.select("[data-lyrics-container='true']");
    sections.select(LYRICS_DECORATIONS).remove();
    let mut text = String::new();
    for section in sections.nodes() {
        let verse = section.formatted_text();
        let verse = verse.trim();
        if !verse.is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(verse);
        }
    }
    if text.is_empty() {
        text = document
            .select("script")
            .nodes()
            .iter()
            .find_map(|script| embedded_lyrics(&script.text()))?;
    }
    (!text.is_empty() && text.len() <= TEXT_LIMIT && !text.contains('\0')).then_some(text)
}

const LYRICS_DECORATIONS: &str =
    "[data-exclude-from-selection], [class*='LyricsHeader'], [class*='LyricsFooter'], script, style, button";

fn embedded_lyrics(script: &str) -> Option<String> {
    let (_, assignment) = script.split_once("window.__PRELOADED_STATE__")?;
    let expression = assignment.trim_start().strip_prefix('=')?.trim_start();
    let argument = expression.strip_prefix("JSON.parse(")?.trim_start();
    let mut chars = argument.chars();
    let quote = chars.next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }

    // Translate the string literal to a JSON string without executing the page's script.
    let mut literal = String::from("\"");
    loop {
        match chars.next()? {
            ch if ch == quote => break,
            '"' => literal.push_str("\\\""),
            '\\' => match chars.next()? {
                '\'' => literal.push('\''),
                'x' => {
                    literal.push_str("\\u00");
                    literal.push(chars.next()?);
                    literal.push(chars.next()?);
                }
                ch => {
                    literal.push('\\');
                    literal.push(ch);
                }
            },
            ch => literal.push(ch),
        }
    }
    if !chars.as_str().trim_start().starts_with(')') {
        return None;
    }
    literal.push('"');
    let json = serde_json::from_str::<String>(&literal).ok()?;
    let state = serde_json::from_str::<serde_json::Value>(&json).ok()?;
    let html = state.pointer("/songPage/lyricsData/body/html")?.as_str()?;
    let fragment = Document::fragment(html);
    fragment.select(LYRICS_DECORATIONS).remove();
    Some(fragment.formatted_text().trim().to_owned())
}

async fn read_bounded(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| GeniusError::Network)? {
        if data.len() + chunk.len() > limit {
            return Err(GeniusError::Service);
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_lyrics_page_keeps_the_song_link_in_the_error() {
        let url = Url::parse("https://genius.com/Example-song-lyrics").unwrap();
        let error = lyrics_page_available(StatusCode::FORBIDDEN, url.clone()).unwrap_err();
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({ "pageUnavailable": { "url": url.as_str() } })
        );
    }

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
        assert_eq!(
            results[0].url.as_str(),
            "https://genius.com/Artist-song-lyrics"
        );
        assert!(results[0].match_rank("Song", &["artist".into()]).is_some());
        assert!(results[0]
            .match_rank("Song", &["another artist".into()])
            .is_none());
    }

    #[test]
    fn finds_the_genius_translation_with_full_collaboration_credits() {
        let data = r#"{"response":{"hits":[
            {"type":"song","result":{"id":1,"title":"Общество Мертвых Поэтов (Dead Poets Society Remix)","artist_names":"VILLIAN & Aarne","url":"https://genius.com/example-remix-lyrics"}},
            {"type":"song","result":{"id":2,"title":"Общество Мертвых Поэтов (Dead Poets Society)","artist_names":"VILLIAN & Other","primary_artist":{"name":"VILLIAN"},"url":"https://genius.com/example-other-lyrics"}},
            {"type":"song","result":{"id":3,"title":"Общество Мертвых Поэтов (Dead Poets Society)","artist_names":"VILLIAN & Aarne","url":"https://genius.com/example-lyrics"}}
        ]}}"#;
        let songs = parse_results(data.as_bytes()).unwrap();
        let artists = ["VILLIAN".into(), "Aarne".into()];
        let title = "Общество Мёртвых Поэтов";
        assert!(songs[0].match_rank(title, &artists).is_none());
        assert!(songs[1].match_rank(title, &artists).is_none());
        assert_eq!(
            best_song(songs, title, &artists, &|_| true).unwrap().id,
            Some(3)
        );
    }

    #[test]
    fn prefers_the_exact_title_to_an_earlier_genius_translation() {
        let data = r#"{"response":{"hits":[
            {"type":"song","result":{"id":1,"title":"Песня (Song)","artist_names":"Artist","url":"https://genius.com/example-translation-lyrics"}},
            {"type":"song","result":{"id":2,"title":"Песня","artist_names":"Artist","url":"https://genius.com/example-lyrics"}}
        ]}}"#;
        let song = best_song(
            parse_results(data.as_bytes()).unwrap(),
            "Песня",
            &["Artist".into()],
            &|_| true,
        )
        .unwrap();
        assert_eq!(song.id, Some(2));
    }

    #[test]
    fn extracts_lyrics_as_plain_text_with_lines_and_verses() {
        let page = r#"<h1>Unrelated title</h1>
            <div data-lyrics-container="true"><div class="LyricsHeader__Title">Credits</div>[Verse]<br>First &amp; <a href="/note">second</a><br>Third<button>Share</button></div>
            <div data-lyrics-container="true">Fourth<br>Fifth<div data-exclude-from-selection="true">Embed</div></div>"#;
        assert_eq!(
            extract_lyrics(page).as_deref(),
            Some("[Verse]\nFirst & second\nThird\n\nFourth\nFifth")
        );
    }

    #[test]
    fn extracts_lyrics_from_the_public_pages_embedded_data() {
        let page = r#"<script>window.__PRELOADED_STATE__ = JSON.parse('{ "songPage": { "lyricsData": { "body": { "html": "[Verse]<br>It\'s \x26 more" } } } }');</script>"#;
        assert_eq!(
            extract_lyrics(page).as_deref(),
            Some("[Verse]\nIt's & more")
        );
    }
}
