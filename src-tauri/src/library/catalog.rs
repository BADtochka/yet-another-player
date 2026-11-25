use std::{collections::HashSet, time::Duration};

use reqwest::{Client, Response, Url};
use rusqlite::{params, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::{ipc::Response as IpcResponse, AppHandle, Manager, State};
use tokio::{
    sync::{Mutex, Semaphore},
    time::{sleep_until, Instant},
};

use super::{connect, user_error, Library, LibraryResult};
use crate::external_tools::{
    enabled::tool_enabled,
    install::Tool,
    ommr::{normalize_url, Configuration, LocalServer, MetadataSource, Mode},
};

mod scan;
pub(crate) mod storage;
pub(super) use scan::warm_scan;

pub(super) static ITUNES_NEXT: Mutex<Option<Instant>> = Mutex::const_new(None);
pub(super) static BRAINZ_NEXT: Mutex<Option<Instant>> = Mutex::const_new(None);
const CATALOG_LIMIT: usize = 4 * 1024 * 1024;
const ARTIST_PROFILE_VERSION: u8 = 4;

#[derive(Clone, Copy, Deserialize, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Provider {
    Itunes,
    MusicBrainz,
    Deezer,
    JioSaavn,
}

#[derive(Clone, Deserialize, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseRef {
    provider: Provider,
    id: String,
}

impl ReleaseRef {
    fn new(provider: Provider, id: String) -> Option<Self> {
        let valid = match provider {
            Provider::MusicBrainz => {
                id.len() == 36
                    && id.bytes().enumerate().all(|(index, ch)| {
                        if [8, 13, 18, 23].contains(&index) {
                            ch == b'-'
                        } else {
                            ch.is_ascii_hexdigit()
                        }
                    })
            }
            _ => !id.is_empty() && id.len() < 20 && id.bytes().all(|ch| ch.is_ascii_digit()),
        };
        valid.then_some(Self { provider, id })
    }

    fn valid(&self) -> bool {
        Self::new(self.provider, self.id.clone()).is_some()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseSummary {
    reference: ReleaseRef,
    #[serde(default)]
    alternates: Vec<ReleaseRef>,
    title: String,
    kind: ReleaseKind,
    date: Option<String>,
    artwork: Option<String>,
}

#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ReleaseKind {
    Album,
    Single,
    Ep,
    Other,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtistProfile {
    avatar: Option<String>,
    biography: Option<String>,
    location: Option<String>,
    active_since: Option<String>,
    genres: Vec<String>,
    releases: Vec<ReleaseSummary>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Release {
    title: String,
    artist: String,
    date: Option<String>,
    artwork: Option<String>,
    #[serde(default)]
    genres: Vec<String>,
    tracks: Vec<ReleaseTrack>,
    complete: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseTrack {
    title: String,
    artist: String,
    disc: Option<u32>,
    position: Option<u32>,
    duration_ms: Option<u64>,
}

fn client(local: bool) -> reqwest::Result<Client> {
    let builder = Client::builder()
        .user_agent(concat!(
            "yet-another-player/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/BADtochka/yet-another-player)"
        ))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(15));
    if local {
        builder.no_proxy().build()
    } else {
        builder.build()
    }
}

async fn bounded(mut response: Response, limit: usize) -> LibraryResult<Vec<u8>> {
    response = response.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > limit {
            return Err("Ответ каталога слишком большой".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn json<T: DeserializeOwned>(response: Response) -> LibraryResult<T> {
    Ok(serde_json::from_slice(
        &bounded(response, CATALOG_LIMIT).await?,
    )?)
}

async fn pace(next: &Mutex<Option<Instant>>, interval: Duration) {
    let mut next = next.lock().await;
    if let Some(at) = *next {
        sleep_until(at).await;
    }
    *next = Some(Instant::now() + interval);
}

async fn itunes<T: DeserializeOwned>(
    client: &Client,
    endpoint: &str,
    query: &[(&str, &str)],
) -> LibraryResult<T> {
    pace(&ITUNES_NEXT, Duration::from_millis(3100)).await;
    json(
        client
            .get(format!("https://itunes.apple.com/{endpoint}"))
            .query(query)
            .send()
            .await?,
    )
    .await
}

async fn brainz<T: DeserializeOwned>(
    client: &Client,
    route: &str,
    query: &[(&str, &str)],
) -> LibraryResult<T> {
    pace(&BRAINZ_NEXT, Duration::from_millis(1200)).await;
    let request = || {
        client
            .get(format!("https://musicbrainz.org/ws/2/{route}"))
            .query(&[("fmt", "json")])
            .query(query)
            .send()
    };
    let mut response = request().await?;
    if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        pace(&BRAINZ_NEXT, Duration::from_millis(1200)).await;
        response = request().await?;
    }
    json(response).await
}

pub(super) fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .map(|ch| if ch == 'ё' { 'е' } else { ch })
        .collect()
}

pub(super) fn release_title(input: &str) -> (&str, ReleaseKind) {
    let input = input.trim();
    let lower = input.to_lowercase();
    if lower.ends_with(" - single") {
        (&input[..input.len() - 9], ReleaseKind::Single)
    } else if lower.ends_with(" - ep") {
        (&input[..input.len() - 5], ReleaseKind::Ep)
    } else {
        (input, ReleaseKind::Album)
    }
}

pub(super) fn release_key(title: &str) -> String {
    let title = release_title(title).0;
    if let Some((base, suffix)) = title
        .strip_suffix(')')
        .and_then(|title| title.rsplit_once('('))
    {
        let cyrillic = |ch: char| ('\u{0400}'..='\u{052f}').contains(&ch);
        let edition = suffix.to_ascii_lowercase();
        if base.chars().any(cyrillic)
            && suffix.chars().any(|ch| ch.is_ascii_alphabetic())
            && suffix.is_ascii()
            && ![
                "live",
                "remix",
                "edit",
                "deluxe",
                "remaster",
                "acoustic",
                "version",
                "instrumental",
                "feat",
                "slowed",
                "sped up",
                "demo",
            ]
            .iter()
            .any(|word| edition.contains(word))
        {
            return normalized(base);
        }
    }
    normalized(title)
}

fn credits_artist(credits: &str, artist: &str) -> bool {
    if normalized(credits) == normalized(artist) {
        return true;
    }
    let names: HashSet<_> = credits.split(['&', ',', ';']).map(normalized).collect();
    artist
        .split(['&', ',', ';'])
        .map(normalized)
        .all(|name| !name.is_empty() && names.contains(&name))
}

static ARTWORK_DOWNLOADS: Semaphore = Semaphore::const_new(4);

pub(super) async fn cache_artwork(database: &std::path::Path, source: &str) -> bool {
    let Ok(mut url) = Url::parse(source) else {
        return false;
    };
    let archive = url.host_str() == Some("coverartarchive.org")
        && url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none();
    if !archive && safe_image(Some(source.to_owned())).is_none() {
        return false;
    }

    let Ok(_download) = ARTWORK_DOWNLOADS.acquire().await else {
        return false;
    };
    let cached = connect(database).and_then(|db| {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM resolver_artwork WHERE url=?1)",
            [source],
            |row| row.get::<_, bool>(0),
        )
    });
    if let Ok(true) = cached {
        return true;
    }

    let Ok(client) = client(false) else {
        return false;
    };
    let result = async {
        if url
            .host_str()
            .is_some_and(|host| host.ends_with(".mzstatic.com"))
        {
            let path = url.path();
            if let Some((prefix, suffix)) = path.rsplit_once('/') {
                if let Some(extension) = suffix.strip_prefix("100x100bb.") {
                    url.set_path(&format!("{prefix}/600x600bb.{extension}"));
                }
            }
        }
        let mut response = client.get(url.clone()).send().await?;
        if archive {
            for _ in 0..3 {
                if !response.status().is_redirection() {
                    break;
                }
                let Some(target) = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|location| url.join(location.to_str().ok()?).ok())
                    .filter(|target| {
                        target.scheme() == "https"
                            && target.username().is_empty()
                            && target.password().is_none()
                            && target.port().is_none()
                            && target.host_str().is_some_and(|host| {
                                host == "archive.org" || host.ends_with(".archive.org")
                            })
                    })
                else {
                    return Err("Недопустимый адрес обложки".into());
                };
                url = target;
                response = client.get(url.clone()).send().await?;
            }
        } else if !response.status().is_success() && url.as_str() != source {
            response = client.get(source).send().await?;
        }
        let bytes = bounded(response, 8 * 1024 * 1024).await?;
        if !matches!(
            image::guess_format(&bytes),
            Ok(image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP)
        ) {
            return Err("Неподдерживаемый формат обложки".into());
        }
        storage::save_artwork(database, source, &bytes)
    }
    .await;
    result.unwrap_or(false)
}

#[tauri::command]
pub(crate) async fn library_album_artwork_image(
    state: State<'_, Library>,
    url: String,
    primary: MetadataSource,
    album_id: Option<String>,
) -> Result<IpcResponse, String> {
    if url.len() > 2048 {
        return Ok(IpcResponse::new(Vec::<u8>::new()));
    }
    let database = state.database_path();
    let cached = || -> LibraryResult<Option<Vec<u8>>> {
        Ok(connect(database)?
            .query_row(
                "SELECT bytes FROM resolver_artwork WHERE url=?1",
                [&url],
                |row| row.get(0),
            )
            .optional()?)
    };
    let mut bytes = cached().map_err(user_error)?;
    if bytes.is_none() && primary != MetadataSource::Off && cache_artwork(database, &url).await {
        bytes = cached().map_err(user_error)?;
    }
    if let Some(id) = album_id.filter(|id| valid_text(id) && bytes.is_some()) {
        connect(database)
            .and_then(|db| {
                db.execute(
                    "INSERT OR REPLACE INTO release_artwork (album_id,url)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM resolver_artwork WHERE url=?2)
             AND EXISTS (SELECT 1 FROM track_index WHERE album_id=?1)",
                    params![id, url],
                )
            })
            .map_err(|error| user_error(error.into()))?;
    }
    Ok(IpcResponse::new(bytes.unwrap_or_default()))
}

fn safe_image(value: Option<String>) -> Option<String> {
    let url = Url::parse(value.as_deref()?).ok()?;
    let host = url.host_str()?;
    let trusted = host.ends_with(".mzstatic.com")
        || host.ends_with(".dzcdn.net")
        || matches!(
            host,
            "images.genius.com"
                | "upload.wikimedia.org"
                | "commons.wikimedia.org"
                | "lh3.googleusercontent.com"
                | "yt3.googleusercontent.com"
                | "i.ytimg.com"
                | "c.saavncdn.com"
        );
    (trusted
        && url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none())
    .then(|| url.to_string())
}

fn date(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.split('T').next().unwrap_or_default().to_owned())
        .filter(|value| !value.is_empty())
}

async fn ommr_endpoint(app: &AppHandle, config: &Configuration) -> Result<Option<String>, String> {
    if !tool_enabled(app, Tool::Ommr) {
        return Ok(None);
    }
    match config.mode {
        Mode::Off => Ok(None),
        Mode::Remote => normalize_url(config.url.as_deref().unwrap_or_default()).map(Some),
        Mode::Local => app.state::<LocalServer>().endpoint(app).await,
    }
}

#[derive(Deserialize)]
struct AppleResults {
    results: Vec<AppleItem>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AppleItem {
    wrapper_type: String,
    artist_id: Option<u64>,
    artist_name: String,
    artist_link_url: Option<String>,
    collection_id: Option<u64>,
    collection_name: String,
    release_date: Option<String>,
    artwork_url100: Option<String>,
    track_name: String,
    disc_number: Option<u32>,
    track_number: Option<u32>,
    track_count: Option<usize>,
    track_time_millis: Option<u64>,
    primary_genre_name: Option<String>,
}

impl AppleItem {
    fn summary(self) -> Option<ReleaseSummary> {
        let (name, mut kind) = release_title(&self.collection_name);
        if kind == ReleaseKind::Album && self.track_count == Some(1) {
            kind = ReleaseKind::Single;
        }
        if name.is_empty() {
            return None;
        }
        Some(ReleaseSummary {
            reference: ReleaseRef::new(Provider::Itunes, self.collection_id?.to_string())?,
            alternates: Vec::new(),
            title: name.to_owned(),
            kind,
            date: date(self.release_date),
            artwork: safe_image(self.artwork_url100),
        })
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SearchCategory {
    Artists,
    Albums,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum CatalogSearchResult {
    Artist {
        id: u64,
        name: String,
        genre: Option<String>,
    },
    Album {
        #[serde(flatten)]
        release: ReleaseSummary,
        artist: String,
    },
}

#[tauri::command]
pub(crate) async fn library_search_catalog(
    query: String,
    category: SearchCategory,
) -> Result<Vec<CatalogSearchResult>, String> {
    let query = query.trim();
    if !valid_text(query) || query.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let entity = match category {
        SearchCategory::Artists => "musicArtist",
        SearchCategory::Albums => "album",
    };
    let client = client(false).map_err(|error| user_error(error.into()))?;
    let found: AppleResults = itunes(
        &client,
        "search",
        &[
            ("term", query),
            ("media", "music"),
            ("entity", entity),
            ("limit", "24"),
        ],
    )
    .await
    .map_err(user_error)?;
    let mut seen = HashSet::new();
    Ok(found
        .results
        .into_iter()
        .filter_map(|item| match category {
            SearchCategory::Artists
                if item.wrapper_type == "artist" && valid_text(&item.artist_name) =>
            {
                let id = item.artist_id?;
                seen.insert(id).then_some(CatalogSearchResult::Artist {
                    id,
                    name: item.artist_name,
                    genre: item.primary_genre_name,
                })
            }
            SearchCategory::Albums
                if item.wrapper_type == "collection" && valid_text(&item.collection_name) =>
            {
                if !seen.insert(item.collection_id?) {
                    return None;
                }
                let artist = item.artist_name.clone();
                item.summary()
                    .filter(|release| release.kind == ReleaseKind::Album)
                    .map(|release| CatalogSearchResult::Album { release, artist })
            }
            _ => None,
        })
        .collect())
}

struct AppleArtist {
    releases: Vec<ReleaseSummary>,
    genre: Option<String>,
    page: Option<Url>,
    page_data: Option<AppleArtistPage>,
}

#[derive(Default)]
struct AppleArtistPage {
    avatar: Option<String>,
    releases: Vec<ReleaseSummary>,
    tracks: Vec<String>,
}

impl AppleItem {
    fn artist_page(&self) -> Option<Url> {
        let id = self.artist_id?;
        let url = Url::parse(self.artist_link_url.as_deref()?).ok()?;
        (url.scheme() == "https"
            && url.host_str() == Some("music.apple.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.path().contains("/artist/")
            && url
                .path()
                .rsplit('/')
                .next()
                .and_then(|last| last.parse::<u64>().ok())
                == Some(id))
        .then_some(url)
    }
}

fn apple_artist_from_tracks<'a>(
    candidates: &'a [AppleItem],
    name: &str,
    tracks: &[AppleItem],
) -> Option<&'a AppleItem> {
    let mut primary = candidates.iter().filter(|artist| {
        tracks.iter().any(|track| {
            track.artist_id == artist.artist_id && credits_artist(&track.artist_name, name)
        })
    });
    if let Some(artist) = primary.next() {
        return primary.next().is_none().then_some(artist);
    }

    let name_key = normalized(name);
    let featured_credits: HashSet<_> = tracks
        .iter()
        .filter_map(|track| {
            let genre = track.primary_genre_name.as_deref()?;
            let (_, suffix) = track.track_name.strip_suffix(')')?.rsplit_once(" (")?;
            let (kind, credit) = suffix.split_once(' ')?;
            matches!(
                kind.to_ascii_lowercase().as_str(),
                "feat." | "feat" | "ft." | "ft" | "featuring"
            )
            .then_some((credit, genre))
        })
        .flat_map(|(credit, genre)| {
            credit
                .split(['&', ',', ';'])
                .map(move |name| (name.trim(), genre))
        })
        .filter(|(credit, _)| normalized(credit) == name_key)
        .collect();
    // The track's artist ID points to the lead. For a featured credit, require
    // one artist with both the credited spelling and the track's genre.
    let mut featured = candidates.iter().filter(|artist| {
        artist
            .primary_genre_name
            .as_deref()
            .is_some_and(|genre| featured_credits.contains(&(artist.artist_name.trim(), genre)))
    });
    featured.next().filter(|_| featured.next().is_none())
}

async fn apple_artist(
    client: &Client,
    name: &str,
    samples: &[String],
) -> LibraryResult<Option<AppleArtist>> {
    let found: AppleResults = itunes(
        client,
        "search",
        &[("term", name), ("entity", "musicArtist"), ("limit", "25")],
    )
    .await?;
    let mut seen = HashSet::new();
    let found: Vec<_> = found
        .results
        .into_iter()
        .filter(|artist| {
            normalized(&artist.artist_name) == normalized(name)
                && artist.artist_id.is_some_and(|id| seen.insert(id))
        })
        .collect();
    let mut exact = found
        .iter()
        .filter(|artist| artist.artist_name.trim() == name.trim());
    let exact = exact.next().filter(|_| exact.next().is_none());
    let mut lookup_error = None;
    let mut artist = if found.len() == 1 {
        found.first()
    } else if !found.is_empty() && !samples.is_empty() {
        let mut tracks = Vec::new();
        for sample in samples.iter().take(2) {
            let term = format!("{name} {sample}");
            let result: LibraryResult<AppleResults> = itunes(
                client,
                "search",
                &[("term", &term), ("entity", "song"), ("limit", "25")],
            )
            .await;
            match result {
                Ok(result) => {
                    let title = normalized(sample);
                    tracks.extend(
                        result
                            .results
                            .into_iter()
                            .filter(|item| normalized(&item.track_name) == title),
                    );
                }
                Err(reason) => lookup_error = Some(reason),
            }
        }
        apple_artist_from_tracks(&found, name, &tracks)
            .or_else(|| tracks.is_empty().then_some(exact).flatten())
    } else {
        exact
    };
    let mut page_data = None;
    if artist.is_none() && !samples.is_empty() && (2..=4).contains(&found.len()) {
        let sample_keys: HashSet<_> = samples.iter().map(|sample| release_key(sample)).collect();
        for candidate in &found {
            let Some(url) = candidate.artist_page() else {
                continue;
            };
            let Ok(page) = apple_artist_page(client, url).await else {
                continue;
            };
            if page
                .releases
                .iter()
                .any(|release| sample_keys.contains(&release_key(&release.title)))
                || page
                    .tracks
                    .iter()
                    .any(|track| sample_keys.contains(&release_key(track)))
            {
                if artist.is_some() {
                    return Ok(None);
                }
                artist = Some(candidate);
                page_data = Some(page);
            }
        }
    }
    let Some(artist) = artist else {
        return lookup_error.map_or(Ok(None), Err);
    };
    let Some(id) = artist.artist_id else {
        return Ok(None);
    };
    let result: AppleResults = itunes(
        client,
        "lookup",
        &[
            ("id", &id.to_string()),
            ("entity", "album"),
            ("limit", "200"),
        ],
    )
    .await?;
    let releases = result
        .results
        .into_iter()
        .filter(|item| {
            item.wrapper_type == "collection"
                && (item.artist_id == Some(id) || credits_artist(&item.artist_name, name))
        })
        .filter_map(AppleItem::summary)
        .collect();
    Ok(Some(AppleArtist {
        releases,
        genre: artist.primary_genre_name.clone(),
        page: artist.artist_page(),
        page_data,
    }))
}

async fn apple_artist_page(client: &Client, page: Url) -> LibraryResult<AppleArtistPage> {
    let page = bounded(client.get(page).send().await?, CATALOG_LIMIT).await?;
    Ok(parse_apple_artist_page(&String::from_utf8(page)?))
}

fn parse_apple_artist_page(html: &str) -> AppleArtistPage {
    let document = dom_query::Document::from(html);
    let image = document
        .select("script[type='application/ld+json']")
        .nodes()
        .iter()
        .filter_map(|script| serde_json::from_str::<serde_json::Value>(&script.text()).ok())
        .find(|data| data["@type"] == "MusicGroup")
        .and_then(|data| data["image"].as_str().map(str::to_owned));
    let mut page = AppleArtistPage {
        avatar: safe_image(image),
        ..Default::default()
    };
    let json = document.select("script#serialized-server-data").text();
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&json) else {
        return page;
    };
    let Some(sections) = data
        .pointer("/data/0/data/sections")
        .and_then(|value| value.as_array())
    else {
        return page;
    };
    for section in sections {
        let marker = section["id"]
            .as_str()
            .and_then(|id| id.split_once(".marker."))
            .map(|(_, marker)| marker);
        let Some(items) = section["items"].as_array() else {
            continue;
        };
        match marker {
            Some(marker)
                if marker.starts_with("Albums-") || marker.starts_with("ArtistSingles-") =>
            {
                page.releases.extend(items.iter().filter_map(|item| {
                    let title = item.pointer("/titleLinks/0/title")?.as_str()?;
                    let year = item
                        .pointer("/subtitleLinks/0/title")
                        .and_then(|value| value.as_str());
                    apple_page_release(item, title, year)
                }));
            }
            Some(marker) if marker.starts_with("shelfgroup.FeaturedContent.TopSongs-") => {
                for item in items {
                    if let Some(title) = item
                        .pointer("/featuredContent/detailText/title")
                        .and_then(|value| value.as_str())
                    {
                        let latest = &item["featuredContent"];
                        let year = latest
                            .pointer("/detailText/headline")
                            .and_then(|value| value.as_str())
                            .and_then(|headline| headline.split_whitespace().last());
                        page.releases
                            .extend(apple_page_release(latest, title, year));
                    }
                    if let Some(tracks) = item["tracks"].as_array() {
                        page.tracks.extend(
                            tracks
                                .iter()
                                .filter_map(|track| track["title"].as_str().map(str::to_owned)),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    page
}

fn apple_page_release(
    item: &serde_json::Value,
    title: &str,
    year: Option<&str>,
) -> Option<ReleaseSummary> {
    let descriptor = &item["contentDescriptor"];
    if descriptor["kind"] != "album" || !valid_text(title) {
        return None;
    }
    let reference = ReleaseRef::new(
        Provider::Itunes,
        descriptor
            .pointer("/identifiers/storeAdamID")?
            .as_str()?
            .into(),
    )?;
    let (title, kind) = release_title(title);
    let artwork = item
        .pointer("/artwork/dictionary/url")
        .and_then(|value| value.as_str())
        .map(|url| {
            url.replace("{w}", "600")
                .replace("{h}", "600")
                .replace("{c}", "bb")
                .replace("{f}", "jpg")
        });
    Some(ReleaseSummary {
        reference,
        alternates: Vec::new(),
        title: title.to_owned(),
        kind,
        date: year
            .filter(|year| year.len() == 4 && year.bytes().all(|ch| ch.is_ascii_digit()))
            .map(str::to_owned),
        artwork: safe_image(artwork),
    })
}

async fn apple_profile_releases(
    client: &Client,
    mut artist: AppleArtist,
) -> (Vec<ReleaseSummary>, Option<String>, Option<String>) {
    let page = match artist.page_data.take() {
        Some(page) => page,
        None => match artist.page.take() {
            Some(url) => apple_artist_page(client, url).await.unwrap_or_default(),
            None => AppleArtistPage::default(),
        },
    };
    // Resolve missing collections in one request for full dates and release types.
    let ids = page
        .releases
        .iter()
        .filter(|page| {
            !artist
                .releases
                .iter()
                .any(|item| item.reference == page.reference)
        })
        .map(|item| item.reference.id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(60)
        .collect::<Vec<_>>()
        .join(",");
    if !ids.is_empty() {
        let details: LibraryResult<AppleResults> = itunes(client, "lookup", &[("id", &ids)]).await;
        if let Ok(details) = details {
            artist.releases.extend(
                details
                    .results
                    .into_iter()
                    .filter(|item| item.wrapper_type == "collection")
                    .filter_map(AppleItem::summary),
            );
        }
    }
    (
        combine_releases(artist.releases, page.releases),
        artist.genre,
        page.avatar,
    )
}

fn apple_album_artwork(
    results: AppleResults,
    artist: &str,
    title: &str,
    year: Option<&str>,
    kind: Option<ReleaseKind>,
) -> Option<String> {
    results
        .results
        .into_iter()
        .filter(|item| {
            item.wrapper_type == "collection" && credits_artist(&item.artist_name, artist)
        })
        .filter_map(AppleItem::summary)
        .filter(|album| {
            release_key(&album.title) == title
                && year.is_none_or(|year| {
                    album.date.as_deref().and_then(|date| date.get(..4)) == Some(year)
                })
        })
        .filter_map(|album| {
            album
                .artwork
                .map(|artwork| (Some(album.kind) == kind, artwork))
        })
        .min_by_key(|(matched_kind, _)| !matched_kind)
        .map(|(_, artwork)| artwork)
}

#[derive(Deserialize)]
struct BrainzArtists {
    artists: Vec<BrainzArtist>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BrainzArtist {
    id: String,
    name: String,
    area: Option<BrainzArea>,
    life_span: Option<BrainzLife>,
    #[serde(default)]
    tags: Vec<BrainzTag>,
}

#[derive(Deserialize)]
struct BrainzArea {
    name: String,
}

#[derive(Deserialize)]
struct BrainzLife {
    begin: Option<String>,
}

#[derive(Deserialize)]
struct BrainzTag {
    name: String,
    count: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BrainzGroups {
    release_groups: Vec<BrainzGroup>,
    release_group_count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BrainzGroup {
    id: String,
    title: String,
    primary_type: Option<String>,
    first_release_date: Option<String>,
}

#[derive(Deserialize)]
struct BrainzRelations {
    relations: Vec<BrainzRelation>,
}

#[derive(Deserialize)]
struct BrainzRelation {
    #[serde(rename = "type")]
    kind: String,
    url: Option<BrainzUrl>,
}

#[derive(Deserialize)]
struct BrainzUrl {
    resource: String,
}

async fn brainz_avatar(client: &Client, id: &str) -> LibraryResult<Option<String>> {
    if ReleaseRef::new(Provider::MusicBrainz, id.to_owned()).is_none() {
        return Ok(None);
    }
    let data: BrainzRelations =
        brainz(client, &format!("artist/{id}"), &[("inc", "url-rels")]).await?;
    let file = data
        .relations
        .into_iter()
        .filter(|item| item.kind == "image")
        .filter_map(|item| {
            let url = Url::parse(&item.url?.resource).ok()?;
            if url.host_str() != Some("commons.wikimedia.org") {
                return None;
            }
            let title = url.path().strip_prefix("/wiki/File:")?;
            Some(title.replace('_', " "))
        })
        .next();
    let Some(file) = file else { return Ok(None) };
    let response = client
        .get("https://commons.wikimedia.org/w/api.php")
        .query(&[
            ("action", "query"),
            ("titles", &format!("File:{file}")),
            ("prop", "imageinfo"),
            ("iiprop", "url"),
            ("iiurlwidth", "480"),
            ("format", "json"),
            ("formatversion", "2"),
        ])
        .send()
        .await?;
    let data: serde_json::Value = json(response).await?;
    Ok(safe_image(
        data.pointer("/query/pages/0/imageinfo/0/thumburl")
            .and_then(|value| value.as_str())
            .map(str::to_owned),
    ))
}

fn quoted(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

async fn identify_brainz_artist(
    client: &Client,
    name: &str,
    samples: &[String],
) -> LibraryResult<Option<BrainzArtist>> {
    let query = format!("artist:\"{}\"", quoted(name));
    let result: BrainzArtists =
        brainz(client, "artist", &[("query", &query), ("limit", "25")]).await?;
    let mut exact: Vec<_> = result
        .artists
        .into_iter()
        .filter(|artist| {
            normalized(&artist.name) == normalized(name)
                && ReleaseRef::new(Provider::MusicBrainz, artist.id.clone()).is_some()
        })
        .collect();
    if exact.len() > 1 && !samples.is_empty() {
        let names = samples
            .iter()
            .take(4)
            .map(|sample| format!("recording:\"{}\"", quoted(sample)))
            .collect::<Vec<_>>()
            .join(" OR ");
        let ids = exact
            .iter()
            .map(|artist| format!("arid:{}", artist.id))
            .collect::<Vec<_>>()
            .join(" OR ");
        let query = format!("({names}) AND ({ids})");
        let recordings: BrainzRecordings =
            brainz(client, "recording", &[("query", &query), ("limit", "25")]).await?;
        let matches: HashSet<_> = recordings
            .recordings
            .into_iter()
            .filter(|recording| {
                samples
                    .iter()
                    .any(|sample| release_key(sample) == release_key(&recording.title))
            })
            .flat_map(|recording| recording.artist_credit)
            .filter_map(|credit| credit.artist.map(|artist| artist.id))
            .collect();
        exact.retain(|artist| matches.contains(&artist.id));
    }
    Ok(exact.pop().filter(|_| exact.is_empty()))
}

async fn brainz_artist(
    client: &Client,
    name: &str,
    samples: &[String],
) -> LibraryResult<Option<(BrainzArtist, Vec<ReleaseSummary>)>> {
    let Some(artist) = identify_brainz_artist(client, name, samples).await? else {
        return Ok(None);
    };
    let mut groups = Vec::new();
    loop {
        let offset = groups.len().to_string();
        let result: BrainzGroups = brainz(
            client,
            "release-group",
            &[
                ("artist", &artist.id),
                ("limit", "100"),
                ("offset", &offset),
            ],
        )
        .await?;
        let page_empty = result.release_groups.is_empty();
        groups.extend(result.release_groups);
        if page_empty || groups.len() >= result.release_group_count {
            break;
        }
    }
    let releases = groups
        .into_iter()
        .filter_map(|group| {
            let kind = match group.primary_type.as_deref() {
                Some("Album") => ReleaseKind::Album,
                Some("Single") => ReleaseKind::Single,
                Some("EP") => ReleaseKind::Ep,
                _ => ReleaseKind::Other,
            };
            Some(ReleaseSummary {
                reference: ReleaseRef::new(Provider::MusicBrainz, group.id)?,
                alternates: Vec::new(),
                title: group.title,
                kind,
                date: date(group.first_release_date),
                artwork: None,
            })
        })
        .collect();
    Ok(Some((artist, releases)))
}

#[derive(Deserialize)]
struct OmmrProfiles {
    profiles: Vec<OmmrProfile>,
}
#[derive(Deserialize)]
struct OmmrProfile {
    name: String,
    biography: Option<String>,
    #[serde(default)]
    images: Vec<OmmrImage>,
}
#[derive(Deserialize)]
struct OmmrImage {
    url: String,
}

async fn ommr_artist(
    client: &Client,
    base: &str,
    name: &str,
    sources: &str,
) -> LibraryResult<Option<(Option<String>, Option<String>)>> {
    let mut query = vec![("artist", name), ("limit", "1")];
    if !sources.is_empty() {
        query.push(("sources", sources));
    }
    let profiles: OmmrProfiles = json(
        client
            .get(format!("{base}/v1/artists"))
            .query(&query)
            .send()
            .await?,
    )
    .await?;
    let mut biography = None;
    let mut avatar = None;
    for profile in profiles
        .profiles
        .into_iter()
        .filter(|profile| normalized(&profile.name) == normalized(name))
    {
        biography = biography.or(profile
            .biography
            .filter(|bio| !bio.trim().is_empty())
            .map(|bio| bio.chars().take(6000).collect()));
        avatar = avatar.or_else(|| {
            profile
                .images
                .into_iter()
                .find_map(|image| safe_image(Some(image.url)))
        });
    }
    Ok((biography.is_some() || avatar.is_some()).then_some((biography, avatar)))
}

#[derive(Deserialize)]
struct OmmrResolution {
    track: Option<OmmrResolved>,
}
#[derive(Deserialize)]
struct OmmrResolved {
    album: Option<OmmrAlbumIdentity>,
}
#[derive(Deserialize)]
struct OmmrAlbumIdentity {
    title: String,
    #[serde(default)]
    ids: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct OmmrReleaseResponse {
    album: Option<OmmrRelease>,
}
#[derive(Deserialize)]
struct OmmrRelease {
    title: String,
    release_date: Option<String>,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    artists: Vec<OmmrCredit>,
    #[serde(default)]
    provider_data: serde_json::Value,
    tracks: Vec<OmmrTrack>,
    track_list: OmmrTrackList,
}
#[derive(Deserialize)]
struct OmmrCredit {
    name: String,
}
#[derive(Deserialize)]
struct OmmrTrackList {
    complete: bool,
}
#[derive(Deserialize)]
struct OmmrTrack {
    title: String,
    position: Option<u32>,
    disc_number: Option<u32>,
    duration_ms: Option<u64>,
    #[serde(default)]
    artists: Vec<OmmrCredit>,
}

fn ommr_provider(provider: Provider) -> &'static str {
    match provider {
        Provider::Itunes => "applemusic",
        Provider::MusicBrainz => "musicbrainz",
        Provider::Deezer => "deezer",
        Provider::JioSaavn => "jiosaavn",
    }
}

async fn ommr_release(
    client: &Client,
    base: &str,
    reference: &ReleaseRef,
) -> LibraryResult<Option<Release>> {
    let result: OmmrReleaseResponse = json(
        client
            .get(format!("{base}/v1/albums"))
            .query(&[
                ("provider", ommr_provider(reference.provider)),
                ("id", &reference.id),
                ("limit", "500"),
            ])
            .send()
            .await?,
    )
    .await?;
    Ok(result.album.map(|album| {
        let names = |credits: Vec<OmmrCredit>| {
            credits
                .into_iter()
                .map(|artist| artist.name)
                .collect::<Vec<_>>()
                .join(", ")
        };
        Release {
            title: release_title(&album.title).0.to_owned(),
            artist: names(album.artists),
            date: date(album.release_date),
            artwork: album
                .images
                .into_iter()
                .find_map(|url| safe_image(Some(url))),
            genres: ["primaryGenreName", "genre"]
                .iter()
                .find_map(|key| {
                    album
                        .provider_data
                        .get(key)
                        .and_then(|value| value.as_str())
                })
                .map(str::to_owned)
                .into_iter()
                .collect(),
            complete: album.track_list.complete,
            tracks: album
                .tracks
                .into_iter()
                .map(|track| ReleaseTrack {
                    title: track.title,
                    artist: names(track.artists),
                    disc: track.disc_number,
                    position: track.position,
                    duration_ms: track.duration_ms,
                })
                .collect(),
        }
    }))
}

async fn apple_release(client: &Client, reference: &ReleaseRef) -> LibraryResult<Option<Release>> {
    let response: AppleResults = itunes(
        client,
        "lookup",
        &[("id", &reference.id), ("entity", "song"), ("limit", "200")],
    )
    .await?;
    let Some(id) = reference.id.parse::<u64>().ok() else {
        return Ok(None);
    };
    let (albums, tracks): (Vec<_>, Vec<_>) = response
        .results
        .into_iter()
        .filter(|row| row.collection_id == Some(id))
        .partition(|row| row.wrapper_type == "collection");
    let Some(album) = albums.into_iter().next() else {
        return Ok(None);
    };
    let mut tracks: Vec<_> = tracks
        .into_iter()
        .filter(|row| row.wrapper_type == "track" && !row.track_name.trim().is_empty())
        .map(|track| ReleaseTrack {
            title: track.track_name,
            artist: track.artist_name,
            disc: track.disc_number,
            position: track.track_number,
            duration_ms: track.track_time_millis,
        })
        .collect();
    tracks.sort_by_key(|track| (track.disc.unwrap_or(1), track.position.unwrap_or(u32::MAX)));
    if tracks.is_empty() {
        return Ok(None);
    }
    Ok(Some(Release {
        title: release_title(&album.collection_name).0.to_owned(),
        artist: album.artist_name,
        date: date(album.release_date),
        artwork: safe_image(album.artwork_url100),
        genres: album.primary_genre_name.into_iter().collect(),
        complete: album.track_count.is_some_and(|count| count <= tracks.len()),
        tracks,
    }))
}

#[derive(Deserialize)]
struct BrainzEditions {
    releases: Vec<BrainzEdition>,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BrainzEdition {
    id: String,
    title: String,
    date: Option<String>,
    status: Option<String>,
    #[serde(default)]
    artist_credit: Vec<BrainzCredit>,
    #[serde(default)]
    media: Vec<BrainzMedium>,
    #[serde(default)]
    genres: Vec<BrainzTag>,
    cover_art_archive: Option<BrainzCoverArt>,
}

#[derive(Deserialize)]
struct BrainzCoverArt {
    #[serde(default)]
    front: bool,
}

#[derive(Deserialize)]
struct BrainzCredit {
    name: String,
    artist: Option<BrainzCreditArtist>,
}

#[derive(Deserialize)]
struct BrainzCreditArtist {
    id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BrainzMedium {
    position: Option<u32>,
    track_count: Option<usize>,
    #[serde(default)]
    tracks: Vec<BrainzTrack>,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BrainzTrack {
    title: String,
    position: Option<u32>,
    length: Option<u64>,
    #[serde(default)]
    artist_credit: Vec<BrainzCredit>,
}

impl From<BrainzEdition> for Release {
    fn from(edition: BrainzEdition) -> Self {
        let names = |credits: Vec<BrainzCredit>| {
            credits
                .into_iter()
                .map(|artist| artist.name)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let complete = !edition.media.is_empty()
            && edition.media.iter().all(|disc| {
                disc.track_count
                    .is_some_and(|count| count <= disc.tracks.len())
            });
        Self {
            title: edition.title,
            artist: names(edition.artist_credit),
            date: date(edition.date),
            artwork: edition
                .cover_art_archive
                .filter(|cover| cover.front)
                .and_then(|_| ReleaseRef::new(Provider::MusicBrainz, edition.id))
                .map(|reference| {
                    format!(
                        "https://coverartarchive.org/release/{}/front-500",
                        reference.id
                    )
                }),
            genres: edition
                .genres
                .into_iter()
                .filter(|genre| genre.count > 0)
                .map(|genre| genre.name)
                .collect(),
            tracks: edition
                .media
                .into_iter()
                .flat_map(|disc| {
                    disc.tracks.into_iter().map(move |track| ReleaseTrack {
                        title: track.title,
                        artist: names(track.artist_credit),
                        disc: disc.position,
                        position: track.position,
                        duration_ms: track.length,
                    })
                })
                .collect(),
            complete,
        }
    }
}

async fn brainz_release(client: &Client, reference: &ReleaseRef) -> LibraryResult<Option<Release>> {
    let response: BrainzEditions = brainz(
        client,
        "release",
        &[("release-group", &reference.id), ("limit", "50")],
    )
    .await?;
    let mut editions: Vec<_> = response
        .releases
        .into_iter()
        .filter(|edition| matches!(edition.status.as_deref(), None | Some("Official")))
        .filter(|edition| ReleaseRef::new(Provider::MusicBrainz, edition.id.clone()).is_some())
        .collect();
    editions.sort_by_key(|edition| {
        (
            edition.status.is_none(),
            edition.date.is_none(),
            edition.date.clone(),
        )
    });
    let mut best = None;
    let mut last_error = None;
    for edition in editions.into_iter().take(3) {
        match load_brainz_edition(client, &edition.id).await {
            Ok(release) if release.complete && !release.tracks.is_empty() => {
                return Ok(Some(release))
            }
            Ok(release)
                if !release.tracks.is_empty()
                    && best.as_ref().is_none_or(|other: &Release| {
                        other.tracks.len() < release.tracks.len()
                    }) =>
            {
                best = Some(release)
            }
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
    }
    match (best, last_error) {
        (None, Some(error)) => Err(error),
        (best, _) => Ok(best),
    }
}

async fn load_brainz_edition(client: &Client, id: &str) -> LibraryResult<Release> {
    let edition: BrainzEdition = brainz(
        client,
        &format!("release/{id}"),
        &[("inc", "recordings+artist-credits+genres")],
    )
    .await?;
    Ok(edition.into())
}

#[derive(Deserialize)]
struct BrainzRecordings {
    recordings: Vec<BrainzRecording>,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BrainzRecording {
    title: String,
    artist_credit: Vec<BrainzCredit>,
    #[serde(default)]
    releases: Vec<BrainzEdition>,
}

async fn find_brainz_release(
    client: &Client,
    artist: &str,
    title: &str,
    expected: Option<&str>,
) -> LibraryResult<Option<Release>> {
    let query = format!(
        "recording:\"{}\" AND artist:\"{}\"",
        quoted(title),
        quoted(artist)
    );
    let result: BrainzRecordings =
        brainz(client, "recording", &[("query", &query), ("limit", "20")]).await?;
    let candidate = result
        .recordings
        .into_iter()
        .filter(|recording| {
            normalized(&recording.title) == normalized(title)
                && normalized(
                    &recording
                        .artist_credit
                        .iter()
                        .map(|credit| credit.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ) == normalized(artist)
        })
        .flat_map(|recording| recording.releases)
        .filter(|release| {
            release.status.as_deref() == Some("Official")
                && ReleaseRef::new(Provider::MusicBrainz, release.id.clone()).is_some()
                && expected.is_none_or(|name| name == release_key(&release.title))
        })
        .min_by_key(|release| (release.date.is_none(), release.date.clone()));
    if let Some(candidate) = candidate {
        let release = load_brainz_edition(client, &candidate.id).await?;
        return Ok(confirms_track(&release, artist, title).then_some(release));
    }
    Ok(None)
}

fn read_cache<T: DeserializeOwned>(
    library: &Library,
    key: &str,
) -> LibraryResult<Option<Option<T>>> {
    let data: Option<String> = connect(library.database_path())?
        .query_row(
            "SELECT response FROM resolver_cache WHERE query=?1
            AND (response<>'null' OR checked>=unixepoch()-14400)",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(data.and_then(|data| serde_json::from_str(&data).ok()))
}

fn save_cache<T: Serialize>(library: &Library, key: &str, result: &Option<T>) -> LibraryResult<()> {
    storage::save_record(
        library.database_path(),
        key,
        &serde_json::to_string(result)?,
    )
}

fn config_key(app: &AppHandle, config: &Configuration) -> LibraryResult<String> {
    let mode = if tool_enabled(app, Tool::Ommr) {
        match config.mode {
            Mode::Off => "off",
            Mode::Local => "local",
            Mode::Remote => "remote",
        }
    } else {
        "off"
    };
    Ok(serde_json::to_string(&(
        mode,
        config.url.as_ref(),
        &config.sources,
        config.primary,
        crate::genius::tool_enabled(app),
    ))?)
}

fn valid_text(input: &str) -> bool {
    !input.trim().is_empty() && input.len() <= 512 && !input.chars().any(char::is_control)
}

async fn saved_avatar(app: &AppHandle, library: &Library, url: &str) -> bool {
    use sha2::{Digest, Sha256};

    let Some(canonical) = safe_image(Some(url.to_owned())) else {
        return false;
    };
    let database = library.database_path();
    let saved = connect(database)
        .and_then(|db| {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM resolver_artwork WHERE url=?1)",
                [url],
                |row| row.get(0),
            )
        })
        .unwrap_or(false);
    if saved {
        return true;
    }

    let Ok(cache) = app.path().app_cache_dir() else {
        return false;
    };
    let directory = cache.join("artist-avatars");
    if !tokio::fs::symlink_metadata(&directory)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return false;
    }
    let hash = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    for extension in ["jpg", "png", "webp"] {
        let path = directory.join(format!("{hash}.{extension}"));
        let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
            continue;
        }
        if let Ok(bytes) = tokio::fs::read(&path).await {
            if storage::save_artwork(database, url, &bytes).unwrap_or(false) {
                let _ = tokio::fs::remove_file(&path).await;
                return true;
            }
        }
    }
    false
}

static AVATAR_LOOKUPS: Semaphore = Semaphore::const_new(3);

fn artist_avatar_urls(library: &Library, artist: &str, key: &str) -> LibraryResult<Vec<String>> {
    let db = connect(library.database_path())?;
    let saved: Option<String> = db
        .query_row(
            "SELECT response FROM resolver_cache WHERE query=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    let mut urls = Vec::new();
    let mut statement = db.prepare(
        "SELECT json_extract(response, '$.avatar') FROM resolver_cache
         WHERE query>=?1 AND query<?2 AND json_type(response, '$.avatar')='text'
         ORDER BY checked DESC LIMIT 6",
    )?;
    // New profile matches take precedence, while saved portraits remain usable offline.
    for version in [ARTIST_PROFILE_VERSION, 3] {
        if version == 3 {
            if let Some(url) = saved
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok())
            {
                if !urls.contains(&url) {
                    urls.push(url);
                }
            }
        }
        let prefix = serde_json::to_string(&("artist-profile", version, artist))?;
        let prefix = prefix.trim_end_matches(']');
        let rows = statement
            .query_map(params![format!("{prefix},"), format!("{prefix}-")], |row| {
                row.get::<_, String>(0)
            })?;
        for row in rows {
            let url = row?;
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    Ok(urls)
}

fn artist_avatar_samples(library: &Library, name: &str) -> LibraryResult<Vec<String>> {
    let db = connect(library.database_path())?;
    let mut statement = db.prepare(
        "SELECT DISTINCT json_extract(t.metadata, '$.name') FROM track_artist a
         JOIN track t ON t.id=a.track_id WHERE a.name_key=?1
         AND json_type(t.metadata, '$.name')='text'
         AND EXISTS (SELECT 1 FROM source WHERE track_id=t.id) LIMIT 4",
    )?;
    let rows = statement.query_map([name.trim().to_lowercase()], |row| row.get::<_, String>(0))?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|title| valid_text(title))
        .collect())
}

async fn resolve_artist_avatar(
    app: &AppHandle,
    library: &Library,
    client: &Client,
    name: &str,
    config: &Configuration,
) -> LibraryResult<Option<String>> {
    if let Ok(Some(base)) = ommr_endpoint(app, config).await {
        let ommr_client = self::client(matches!(config.mode, Mode::Local))?;
        if let Ok(Some((_, Some(url)))) =
            ommr_artist(&ommr_client, &base, name, &config.sources()?).await
        {
            return Ok(Some(url));
        }
    }
    let samples = artist_avatar_samples(library, name)?;
    if let Some(title) = samples.first().filter(|_| crate::genius::tool_enabled(app)) {
        if let Ok(Some(token)) = crate::genius::access_token(app).await {
            if let Ok(Some(url)) = crate::genius::artist_avatar(&token, title, name).await {
                if let Some(url) = safe_image(Some(url)) {
                    return Ok(Some(url));
                }
            }
        }
    }
    if let Some(artist) = identify_brainz_artist(client, name, &samples).await? {
        return brainz_avatar(client, &artist.id).await;
    }
    Ok(None)
}

#[tauri::command]
pub(crate) async fn library_artist_avatar(
    app: AppHandle,
    state: State<'_, Library>,
    name: String,
    config: Configuration,
) -> Result<Option<String>, String> {
    if !valid_text(&name) {
        return Ok(None);
    }
    let artist = normalized(&name);
    if artist.is_empty() {
        return Ok(None);
    }
    let result: LibraryResult<_> = async {
        let name_key = name.trim().to_lowercase();
        let key = serde_json::to_string(&("artist-avatar", 1, &name_key))?;
        let urls = artist_avatar_urls(&state, &artist, &key)?;
        for url in &urls {
            if saved_avatar(&app, &state, url).await {
                return Ok(Some(url.clone()));
            }
        }
        if config.primary == MetadataSource::Off {
            return Ok(None);
        }
        let _permit = AVATAR_LOOKUPS.acquire().await?;
        let missing = serde_json::to_string(&(
            "artist-avatar-missing",
            1,
            &name_key,
            config_key(&app, &config)?,
        ))?;
        if read_cache::<String>(&state, &missing)?.is_some() {
            return Ok(None);
        }
        let client = client(false)?;
        if let Some(url) = urls.first() {
            if cache_artwork(state.database_path(), url).await {
                save_cache(&state, &key, &Some(url))?;
                return Ok(Some(url.clone()));
            }
        }
        if let Some(url) = resolve_artist_avatar(&app, &state, &client, &name, &config).await? {
            if cache_artwork(state.database_path(), &url).await {
                save_cache(&state, &key, &Some(&url))?;
                return Ok(Some(url));
            }
        }
        save_cache::<String>(&state, &missing, &None)?;
        Ok(None)
    }
    .await;
    result.map_err(user_error)
}

fn combine_releases(
    apple: Vec<ReleaseSummary>,
    brainz: Vec<ReleaseSummary>,
) -> Vec<ReleaseSummary> {
    let mut merged: Vec<ReleaseSummary> = Vec::new();
    for mut candidate in apple.into_iter().chain(brainz) {
        if let Some(item) = merged.iter_mut().find(|item| {
            item.reference == candidate.reference
                || (release_key(&item.title) == release_key(&candidate.title)
                    && match (item.date.as_deref(), candidate.date.as_deref()) {
                        (Some(first), Some(second)) => {
                            first.get(..4) == second.get(..4)
                                && (item.kind == candidate.kind
                                    || first.len() == 4
                                    || second.len() == 4
                                    || first == second)
                        }
                        _ => true,
                    })
        }) {
            if item.reference != candidate.reference
                && !item.alternates.contains(&candidate.reference)
            {
                item.alternates.push(candidate.reference);
            }
            if item.kind == ReleaseKind::Album && candidate.kind != ReleaseKind::Other {
                item.kind = candidate.kind;
            }
            if normalized(&item.title) != release_key(&item.title)
                && normalized(&candidate.title) == release_key(&candidate.title)
            {
                item.title = candidate.title;
            }
            if item.date.as_ref().map_or(0, String::len)
                < candidate.date.as_ref().map_or(0, String::len)
            {
                item.date = candidate.date.take();
            }
            if item.artwork.is_none() {
                item.artwork = candidate.artwork;
            }
        } else {
            merged.push(candidate);
        }
    }
    merged.sort_by(|left, right| {
        right
            .date
            .cmp(&left.date)
            .then_with(|| left.title.cmp(&right.title))
    });
    merged
}

#[tauri::command]
pub(crate) async fn library_artist_profile(
    app: AppHandle,
    state: State<'_, Library>,
    name: String,
    sample_title: Option<String>,
    sample_titles: Option<Vec<String>>,
    saved_only: Option<bool>,
    config: Configuration,
) -> Result<Option<ArtistProfile>, String> {
    if config.primary == MetadataSource::Off || !valid_text(&name) {
        return Ok(None);
    }
    let samples = sample_title
        .into_iter()
        .chain(sample_titles.into_iter().flatten())
        .filter(|title| valid_text(title))
        .take(4)
        .collect::<Vec<_>>();
    if saved_only == Some(true) {
        for genius in [true, false] {
            let key =
                artist_profile_key(&app, &name, &samples, genius, &config).map_err(user_error)?;
            if let Some(profile) = read_cache::<ArtistProfile>(&state, &key)
                .map_err(user_error)?
                .flatten()
            {
                if let Some(url) = profile.avatar.as_deref() {
                    saved_avatar(&app, &state, url).await;
                }
                return Ok(Some(profile));
            }
        }
        return Ok(None);
    }
    let token = if !samples.is_empty() && crate::genius::tool_enabled(&app) {
        crate::genius::access_token(&app).await.unwrap_or_default()
    } else {
        None
    };
    artist_profile(
        &app,
        &state,
        &name,
        &samples,
        token.as_deref(),
        &config,
        true,
    )
    .await
    .map_err(user_error)
}

fn artist_profile_key(
    app: &AppHandle,
    name: &str,
    samples: &[String],
    genius: bool,
    config: &Configuration,
) -> LibraryResult<String> {
    Ok(serde_json::to_string(&(
        "artist-profile",
        ARTIST_PROFILE_VERSION,
        normalized(name),
        config_key(app, config)?,
        genius,
        samples
            .iter()
            .map(|sample| normalized(sample))
            .collect::<Vec<_>>(),
    ))?)
}

async fn artist_profile(
    app: &AppHandle,
    library: &Library,
    name: &str,
    samples: &[String],
    token: Option<&str>,
    config: &Configuration,
    refresh: bool,
) -> LibraryResult<Option<ArtistProfile>> {
    let key = artist_profile_key(app, name, samples, token.is_some(), config)?;
    let cached = read_cache::<ArtistProfile>(library, &key)?;
    if !refresh {
        if let Some(cached) = cached {
            return Ok(cached);
        }
    }
    let client = client(false)?;
    let sources = config.sources()?;
    let ommr_request = async {
        let Some(base) = ommr_endpoint(app, config).await? else {
            return Ok(None);
        };
        let ommr_client = self::client(matches!(config.mode, Mode::Local))?;
        ommr_artist(&ommr_client, &base, name, &sources).await
    };
    let apple_request = async {
        let Some(apple) = apple_artist(&client, name, samples).await? else {
            return Ok::<_, super::LibraryError>(None);
        };
        Ok(Some(apple_profile_releases(&client, apple).await))
    };
    let (apple, brainz, ommr) = tokio::join!(
        apple_request,
        brainz_artist(&client, name, samples),
        ommr_request
    );
    let succeeded = apple.is_ok() || brainz.is_ok() || ommr.as_ref().is_ok_and(Option::is_some);
    let catalogs_answered = apple.is_ok() && brainz.is_ok();
    let (apple_releases, genre, portrait) = apple.unwrap_or_default().unwrap_or_default();
    let (location, active_since, mut genres, brainz_releases, brainz_id) =
        match brainz.unwrap_or_default() {
            Some((artist, releases)) => (
                artist.area.map(|area| area.name),
                artist.life_span.and_then(|life| life.begin),
                artist
                    .tags
                    .into_iter()
                    .filter(|tag| tag.count > 0)
                    .take(8)
                    .map(|tag| tag.name)
                    .collect::<Vec<_>>(),
                releases,
                Some(artist.id),
            ),
            None => (None, None, Vec::new(), Vec::new(), None),
        };
    if let Some(genre) = genre.filter(|genre| {
        !genres
            .iter()
            .any(|value| normalized(value) == normalized(genre))
    }) {
        genres.push(genre);
    }
    let (biography, ommr_avatar) = ommr.unwrap_or_default().unwrap_or_default();
    let mut avatar = portrait.or(ommr_avatar);
    if avatar.is_none() {
        if let (Some(token), Some(title)) = (token, samples.first().map(String::as_str)) {
            avatar = safe_image(
                crate::genius::artist_avatar(token, title, name)
                    .await
                    .unwrap_or_default(),
            );
        }
    }
    if avatar.is_none() {
        if let Some(id) = brainz_id {
            avatar = brainz_avatar(&client, &id).await.unwrap_or_default();
        }
    }
    let mut releases = combine_releases(apple_releases, brainz_releases);
    if !succeeded && avatar.is_none() {
        return Err("Внешние каталоги артистов недоступны".into());
    }
    let mut previous = cached.flatten();
    if previous.is_none() && refresh {
        let previous_key = artist_profile_key(app, name, samples, token.is_none(), config)?;
        previous = read_cache::<ArtistProfile>(library, &previous_key)?.flatten();
    }
    if let Some(previous) = &mut previous {
        if !catalogs_answered || releases.is_empty() {
            releases = combine_releases(releases, std::mem::take(&mut previous.releases));
        }
        if genres.is_empty() {
            genres = std::mem::take(&mut previous.genres);
        }
    }
    let has_releases = !releases.is_empty();
    let profile = (has_releases
        || !genres.is_empty()
        || location.is_some()
        || biography.is_some()
        || avatar.is_some())
    .then_some(ArtistProfile {
        avatar: avatar.or_else(|| previous.as_mut()?.avatar.take()),
        biography: biography.or_else(|| previous.as_mut()?.biography.take()),
        location: location.or_else(|| previous.as_mut()?.location.take()),
        active_since: active_since.or_else(|| previous.as_mut()?.active_since.take()),
        genres,
        releases,
    });
    if has_releases || catalogs_answered {
        save_cache(library, &key, &profile)?;
    }
    Ok(profile)
}

#[tauri::command]
pub(crate) async fn library_release_details(
    app: AppHandle,
    state: State<'_, Library>,
    reference: ReleaseRef,
    alternates: Option<Vec<ReleaseRef>>,
    config: Configuration,
) -> Result<Option<Release>, String> {
    if config.primary == MetadataSource::Off || !reference.valid() {
        return Ok(None);
    }
    release_from_refs(
        &app,
        &state,
        &config,
        &reference,
        &alternates.unwrap_or_default(),
    )
    .await
    .map_err(user_error)
}

async fn release_from_refs(
    app: &AppHandle,
    library: &Library,
    config: &Configuration,
    reference: &ReleaseRef,
    alternates: &[ReleaseRef],
) -> LibraryResult<Option<Release>> {
    let mut best = None;
    let mut last_error = None;
    let mut seen = HashSet::new();
    for reference in std::iter::once(reference).chain(alternates.iter().take(5)) {
        if !reference.valid() || !seen.insert(reference) {
            continue;
        }
        match release_by_ref(app, library, config, reference).await {
            Ok(Some(release)) if release.complete => return Ok(Some(release)),
            Ok(Some(release))
                if best
                    .as_ref()
                    .is_none_or(|other: &Release| other.tracks.len() < release.tracks.len()) =>
            {
                best = Some(release)
            }
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
    }
    match (best, last_error) {
        (None, Some(error)) => Err(error),
        (best, _) => Ok(best),
    }
}

async fn release_by_ref(
    app: &AppHandle,
    library: &Library,
    config: &Configuration,
    reference: &ReleaseRef,
) -> LibraryResult<Option<Release>> {
    let key = release_cache_key(reference, &config_key(app, config)?)?;
    if let Some(cached) = read_cache(library, &key)? {
        return Ok(cached);
    }
    let result = match reference.provider {
        Provider::Itunes => apple_release(&client(false)?, reference).await?,
        Provider::MusicBrainz => brainz_release(&client(false)?, reference).await?,
        _ => match ommr_endpoint(app, config).await? {
            Some(base) => {
                ommr_release(
                    &client(matches!(config.mode, Mode::Local))?,
                    &base,
                    reference,
                )
                .await?
            }
            None => None,
        },
    }
    .filter(|release| !release.tracks.is_empty());
    save_cache(library, &key, &result)?;
    Ok(result)
}

fn release_cache_key(reference: &ReleaseRef, config: &str) -> LibraryResult<String> {
    let key = ("release-details", 2, reference, config);
    Ok(serde_json::to_string(&key)?)
}

#[tauri::command]
pub(crate) async fn library_find_album_artwork(
    app: AppHandle,
    state: State<'_, Library>,
    artist: String,
    title: String,
    year: Option<String>,
    kind: Option<ReleaseKind>,
    references: Option<Vec<ReleaseRef>>,
    sample: Option<ArtworkSample>,
    excluded: Option<Vec<String>>,
    config: Configuration,
) -> Result<Option<String>, String> {
    if config.primary == MetadataSource::Off || !valid_text(&artist) || !valid_text(&title) {
        return Ok(None);
    }
    let year = year
        .as_deref()
        .filter(|year| year.len() == 4 && year.bytes().all(|ch| ch.is_ascii_digit()));
    let key_title = release_key(&title);
    let excluded = excluded.unwrap_or_default();
    let usable = |url: &String| !excluded.iter().take(12).any(|item| item == url);
    let result: LibraryResult<_> = async {
        if let Some(references) = references {
            let config_id = config_key(&app, &config)?;
            for reference in references
                .iter()
                .take(6)
                .filter(|reference| reference.valid())
            {
                let key = release_cache_key(reference, &config_id)?;
                if let Some(artwork) = read_cache::<Release>(&state, &key)?
                    .flatten()
                    .and_then(|release| release.artwork)
                    .filter(usable)
                {
                    return Ok(Some(artwork));
                }
            }
            if let Some(artwork) =
                resolve_reference_artwork(&app, &state, &config, &references, &usable).await
            {
                return Ok(Some(artwork));
            }
        }
        if let Some(sample) = sample.filter(ArtworkSample::valid) {
            if let Ok(Some(release)) = find_release(
                &app,
                &state,
                &sample.artist,
                &sample.title,
                sample.album.as_deref(),
                &config,
                &[],
            )
            .await
            {
                if let Some(artwork) = release.artwork.filter(usable) {
                    return Ok(Some(artwork));
                }
            }
        }
        let key =
            serde_json::to_string(&("album-artwork", normalized(&artist), &key_title, year, kind))?;
        if let Some(cached) = read_cache::<String>(&state, &key)? {
            return Ok(cached.filter(usable));
        }
        let term = format!("{artist} {title}");
        let results = itunes(
            &client(false)?,
            "search",
            &[("term", &term), ("entity", "album"), ("limit", "24")],
        )
        .await?;
        let artwork = apple_album_artwork(results, &artist, &key_title, year, kind);
        save_cache(&state, &key, &artwork)?;
        Ok(artwork.filter(usable))
    }
    .await;
    result.map_err(user_error)
}

#[derive(Deserialize)]
pub(crate) struct ArtworkSample {
    artist: String,
    title: String,
    album: Option<String>,
}

impl ArtworkSample {
    fn valid(&self) -> bool {
        valid_text(&self.artist)
            && valid_text(&self.title)
            && self.album.as_deref().is_none_or(valid_text)
    }
}

async fn resolve_reference_artwork(
    app: &AppHandle,
    library: &Library,
    config: &Configuration,
    references: &[ReleaseRef],
    usable: &impl Fn(&String) -> bool,
) -> Option<String> {
    for reference in references
        .iter()
        .take(6)
        .filter(|reference| reference.valid())
    {
        if let Ok(Some(release)) = release_by_ref(app, library, config, reference).await {
            if let Some(artwork) = release.artwork.filter(usable) {
                return Some(artwork);
            }
        }
    }
    None
}

#[tauri::command]
pub(crate) async fn library_find_release(
    app: AppHandle,
    state: State<'_, Library>,
    artist: String,
    title: String,
    album: Option<String>,
    config: Configuration,
) -> Result<Option<Release>, String> {
    if config.primary == MetadataSource::Off
        || !valid_text(&artist)
        || !valid_text(&title)
        || album.as_deref().is_some_and(|album| !valid_text(album))
    {
        return Ok(None);
    }
    find_release(
        &app,
        &state,
        &artist,
        &title,
        album.as_deref(),
        &config,
        &[],
    )
    .await
    .map_err(user_error)
}

fn confirms_track(release: &Release, artist: &str, title: &str) -> bool {
    release.tracks.iter().any(|track| {
        release_key(&track.title) == release_key(title)
            && (credits_artist(&track.artist, artist)
                || (track.artist.is_empty() && credits_artist(&release.artist, artist)))
    })
}

async fn find_release(
    app: &AppHandle,
    library: &Library,
    artist: &str,
    title: &str,
    album: Option<&str>,
    config: &Configuration,
    known: &[ReleaseSummary],
) -> LibraryResult<Option<Release>> {
    let key = serde_json::to_string(&(
        "track-release",
        2,
        normalized(artist),
        release_key(title),
        album.map(release_key),
        config_key(app, config)?,
    ))?;
    let cached = read_cache::<Release>(library, &key)?;
    if let Some(Some(release)) = cached {
        return Ok(Some(release));
    }
    let album_key = release_key(album.unwrap_or(title));
    for release in known.iter().filter(|release| {
        release_key(&release.title) == album_key
            && (album.is_some() || release.kind == ReleaseKind::Single)
    }) {
        if let Ok(Some(details)) = release_from_refs(
            app,
            library,
            config,
            &release.reference,
            &release.alternates,
        )
        .await
        {
            if details.complete && confirms_track(&details, artist, title) {
                save_cache(library, &key, &Some(&details))?;
                return Ok(Some(details));
            }
        }
    }
    if matches!(cached, Some(None)) {
        return Ok(None);
    }
    let public_client = client(false)?;
    let expected = album.map(release_key);
    let mut partial = None;
    if let Ok(Some(base)) = ommr_endpoint(app, config).await {
        let check = async {
            let ommr_client = self::client(matches!(config.mode, Mode::Local))?;
            let sources = config.sources()?;
            let mut query = vec![("artist", artist), ("title", title)];
            if let Some(album) = album {
                query.push(("album", album));
            }
            if !sources.is_empty() {
                query.push(("sources", &sources));
            }
            let resolved: OmmrResolution = json(
                ommr_client
                    .get(format!("{base}/v1/resolve"))
                    .query(&query)
                    .send()
                    .await?,
            )
            .await?;
            let Some(identity) = resolved.track.and_then(|track| track.album) else {
                return Ok(None);
            };
            if expected
                .as_ref()
                .is_some_and(|name| *name != release_key(&identity.title))
            {
                return Ok(None);
            }
            for provider in [
                Provider::Itunes,
                Provider::Deezer,
                Provider::JioSaavn,
                Provider::MusicBrainz,
            ] {
                let Some(reference) = identity
                    .ids
                    .get(ommr_provider(provider))
                    .and_then(|id| ReleaseRef::new(provider, id.clone()))
                else {
                    continue;
                };
                if let Ok(Some(release)) = ommr_release(&ommr_client, &base, &reference).await {
                    if confirms_track(&release, artist, title) {
                        if release.complete {
                            return Ok(Some(release));
                        }
                        if partial.as_ref().is_none_or(|existing: &Release| {
                            existing.tracks.len() < release.tracks.len()
                        }) {
                            partial = Some(release);
                        }
                    }
                }
            }
            Ok::<_, super::LibraryError>(None)
        }
        .await;
        if let Ok(Some(release)) = check {
            let result = Some(release);
            save_cache(library, &key, &result)?;
            return Ok(result);
        }
    }
    let apple = async {
        let term = format!("{artist} {title}");
        let result: AppleResults = itunes(
            &public_client,
            "search",
            &[("term", &term), ("entity", "song"), ("limit", "30")],
        )
        .await?;
        let candidate = result
            .results
            .into_iter()
            .find(|row| {
                release_key(&row.track_name) == release_key(title)
                    && credits_artist(&row.artist_name, artist)
                    && expected
                        .as_ref()
                        .is_none_or(|name| *name == release_key(&row.collection_name))
            })
            .and_then(|row| ReleaseRef::new(Provider::Itunes, row.collection_id?.to_string()));
        if let Some(reference) = candidate {
            if let Some(release) = release_by_ref(app, library, config, &reference)
                .await?
                .filter(|release| confirms_track(release, artist, title))
            {
                return Ok(Some(release));
            }
        }
        let primary = artist.split(',').next().unwrap_or(artist).trim();
        if let Some(profile) = apple_artist(&public_client, primary, &[title.to_owned()]).await? {
            for item in profile
                .releases
                .iter()
                .filter(|item| {
                    release_key(&item.title) == album_key
                        && (album.is_some() || item.kind == ReleaseKind::Single)
                })
                .take(3)
            {
                if let Some(release) = release_by_ref(app, library, config, &item.reference)
                    .await?
                    .filter(|release| confirms_track(release, artist, title))
                {
                    return Ok(Some(release));
                }
            }
        }
        Ok::<_, super::LibraryError>(None)
    }
    .await;
    if let Ok(Some(release)) = apple.as_ref() {
        if release.complete {
            save_cache(library, &key, &apple.as_ref().ok().and_then(Option::as_ref))?;
            return apple;
        }
    }
    let brainz = find_brainz_release(&public_client, artist, title, expected.as_deref()).await;
    let any_available = apple.is_ok() || brainz.is_ok() || partial.is_some();
    let result = apple
        .unwrap_or_default()
        .into_iter()
        .chain(brainz.unwrap_or_default())
        .chain(partial)
        .max_by_key(|release| (release.complete, release.tracks.len()));
    if !any_available {
        return Err("Внешний треклист недоступен".into());
    }
    save_cache(library, &key, &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_uses_lead_id_and_disambiguates_the_featured_artist() {
        let candidate = |id, name: &str, genre: &str| AppleItem {
            artist_id: Some(id),
            artist_name: name.into(),
            primary_genre_name: Some(genre.into()),
            ..AppleItem::default()
        };
        let tracks = [AppleItem {
            artist_id: Some(1528208237),
            artist_name: "OSA".into(),
            track_name: "MOON Freestyle (feat. SALUKI)".into(),
            primary_genre_name: Some("Hip-Hop/Rap".into()),
            ..AppleItem::default()
        }];
        let osa = [
            candidate(1581444212, "OSA", "R&B/Soul"),
            candidate(1528208237, "OSA", "Hip-Hop/Rap"),
        ];
        assert_eq!(
            apple_artist_from_tracks(&osa, "OSA", &tracks).and_then(|artist| artist.artist_id),
            Some(1528208237)
        );
        let saluki = [
            candidate(1123097705, "SALUKI", "Prog-Rock/Art Rock"),
            candidate(1450020262, "Saluki", "Hip-Hop/Rap"),
            candidate(1427819350, "SALUKI", "Hip-Hop/Rap"),
        ];
        assert_eq!(
            apple_artist_from_tracks(&saluki, "Saluki", &tracks)
                .and_then(|artist| artist.artist_id),
            Some(1427819350)
        );
    }

    #[tokio::test]
    #[ignore = "uses the public Apple Music and iTunes catalogs"]
    async fn loads_osa_and_saluki_from_shared_local_tracks() {
        let client = client(false).unwrap();
        let tracks = [
            "2X20 (feat. SALUKI)".into(),
            "MOON Freestyle (feat. SALUKI)".into(),
        ];
        for (name, album) in [("OSA", "REPLEXIA"), ("SALUKI", "WILD EA$T")] {
            let artist = apple_artist(&client, name, &tracks).await.unwrap().unwrap();
            assert!(artist.releases.iter().any(|release| release.title == album));
            assert!(apple_profile_releases(&client, artist).await.2.is_some());
        }
    }

    #[test]
    fn artist_page_supplies_shared_singles_and_ignores_recommendations() {
        let html = r#"<script type="application/json" id="serialized-server-data">
            {"data":[{"data":{"sections":[
              {"id":"artist.marker.ArtistSingles-SquareLockupSection","items":[{
                "titleLinks":[{"title":"Претендент На Убийство - Single"}],
                "subtitleLinks":[{"title":"2026"}],
                "contentDescriptor":{"kind":"album","identifiers":{"storeAdamID":"6835955314"}}
              }]},
              {"id":"artist.marker.SimilarArtists-SquareLockupSection","items":[{
                "titleLinks":[{"title":"Unrelated album"}],
                "contentDescriptor":{"kind":"album","identifiers":{"storeAdamID":"1234"}}
              }]}
            ]}}]}
        </script>"#;
        let page = parse_apple_artist_page(html);
        assert_eq!(page.releases.len(), 1);
        let release = &page.releases[0];
        assert_eq!(release.reference.id, "6835955314");
        assert_eq!(release.title, "Претендент На Убийство");
        assert!(release.kind == ReleaseKind::Single);
        assert_eq!(release.date.as_deref(), Some("2026"));
    }

    #[tokio::test]
    #[ignore = "uses the public Apple Music and iTunes catalogs"]
    async fn finds_the_shared_single_on_both_official_artist_pages() {
        let client = client(false).unwrap();
        for (name, samples) in [
            ("Aarne", vec!["Общество Мертвых Поэтов".into()]),
            (
                "VILLIAN",
                vec!["ДИНАСТИЯ".into(), "Общество Мертвых Поэтов".into()],
            ),
        ] {
            let artist = apple_artist(&client, name, &samples)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("No Apple artist profile for {name}"));
            let (releases, _, _) = apple_profile_releases(&client, artist).await;
            let release = releases
                .iter()
                .find(|release| release.reference.id == "6835955314")
                .unwrap_or_else(|| panic!("Joint release absent from {name}"));
            assert!(release.kind == ReleaseKind::Single);
            assert!(release.date.as_deref().is_some_and(|date| date.len() == 10));
        }
    }

    #[test]
    fn artist_avatar_reuses_old_profiles_and_survives_a_catalog_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("library.sqlite3");
        let library = Library::new(database.clone()).unwrap();
        let key = serde_json::to_string(&("artist-avatar", 1, "osa")).unwrap();
        let legacy = "https://images.genius.com/legacy.jpg";
        let preferred = "https://images.genius.com/saved.jpg";
        for (artist, url) in [
            ("osa", legacy),
            ("osaa", "https://images.genius.com/unrelated.jpg"),
        ] {
            let profile_key =
                serde_json::to_string(&("artist-profile", 3, artist, "old settings", false))
                    .unwrap();
            connect(&database)
                .unwrap()
                .execute(
                    "INSERT INTO resolver_cache VALUES (?1, 0, ?2)",
                    params![
                        profile_key,
                        serde_json::json!({ "avatar": url }).to_string()
                    ],
                )
                .unwrap();
        }
        assert_eq!(artist_avatar_urls(&library, "osa", &key).unwrap(), [legacy]);
        save_cache(&library, &key, &Some(preferred)).unwrap();
        drop(library);
        let reopened = Library::new(database).unwrap();
        assert_eq!(
            artist_avatar_urls(&reopened, "osa", &key).unwrap(),
            [preferred, legacy]
        );
    }

    #[test]
    fn album_artwork_matches_artist_release_and_year() {
        let collection = |id: u64, artist: &str, title: &str, year: &str| AppleItem {
            wrapper_type: "collection".into(),
            collection_id: Some(id),
            artist_name: artist.into(),
            collection_name: title.into(),
            release_date: Some(format!("{year}-01-01T00:00:00Z")),
            artwork_url100: Some(format!("https://is1-ssl.mzstatic.com/{id}.jpg")),
            ..AppleItem::default()
        };
        let results = AppleResults {
            results: vec![
                collection(1, "Другой артист", "Зима", "2025"),
                collection(2, "Артист", "Другой альбом", "2025"),
                collection(3, "Артист", "Зима", "2024"),
                collection(4, "Артист", "Зима", "2025"),
                collection(5, "Артист", "Зима - Single", "2025"),
            ],
        };
        assert_eq!(
            apple_album_artwork(
                results,
                "Артист",
                &release_key("Зима"),
                Some("2025"),
                Some(ReleaseKind::Single),
            )
            .as_deref(),
            Some("https://is1-ssl.mzstatic.com/5.jpg")
        );
    }

    #[test]
    fn merges_editions_without_losing_release_categories() {
        let reference = |id: &str| ReleaseRef::new(Provider::Itunes, id.to_owned()).unwrap();
        let apple = vec![ReleaseSummary {
            reference: reference("12"),
            alternates: Vec::new(),
            title: "Пока-пока".into(),
            kind: ReleaseKind::Album,
            date: Some("2024-09-06".into()),
            artwork: Some("https://is1-ssl.mzstatic.com/example.jpg".into()),
        }];
        let brainz = vec![ReleaseSummary {
            reference: ReleaseRef::new(
                Provider::MusicBrainz,
                "00000000-0000-0000-0000-000000000123".into(),
            )
            .unwrap(),
            alternates: Vec::new(),
            title: "пока-пока".into(),
            kind: ReleaseKind::Ep,
            date: Some("2024".into()),
            artwork: None,
        }];
        let merged = combine_releases(apple, brainz);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].kind == ReleaseKind::Ep);
        assert_eq!(merged[0].reference.id, "12");
        assert!(merged[0].artwork.is_some());
        assert_eq!(merged[0].alternates.len(), 1);
    }

    #[test]
    fn combines_translated_titles_but_keeps_distinct_editions_and_one_track_singles() {
        let item = |id, name: &str, track_count| {
            AppleItem {
                collection_id: Some(id),
                collection_name: name.into(),
                release_date: Some("2026-07-24".into()),
                track_count: Some(track_count),
                ..Default::default()
            }
            .summary()
            .unwrap()
        };
        let merged = combine_releases(
            vec![
                item(1, "Красивое Зло (Beautiful Evil)", 7),
                item(2, "Красивое Зло", 7),
                item(3, "Красивое Зло (Live)", 7),
                item(4, "Общество Мертвых Поэтов", 1),
            ],
            Vec::new(),
        );
        assert_eq!(merged.len(), 3);
        let release = merged
            .iter()
            .find(|release| release.title == "Красивое Зло")
            .unwrap();
        assert_eq!(release.alternates[0].id, "2");
        assert!(merged
            .iter()
            .any(|release| release.title == "Общество Мертвых Поэтов"
                && release.kind == ReleaseKind::Single));
    }

    #[test]
    fn cache_retains_release_tracklist_between_library_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("library.sqlite3");
        let library = Library::new(database.clone()).unwrap();
        let release = Release {
            title: "Release".into(),
            artist: "Artist".into(),
            date: None,
            artwork: None,
            genres: Vec::new(),
            tracks: vec![ReleaseTrack {
                title: "Track".into(),
                artist: "Artist".into(),
                disc: Some(1),
                position: Some(2),
                duration_ms: None,
            }],
            complete: true,
        };
        save_cache(&library, "release-test", &Some(release)).unwrap();
        drop(library);
        let library = Library::new(database).unwrap();
        let cached = read_cache::<Release>(&library, "release-test")
            .unwrap()
            .flatten()
            .unwrap();
        assert!(cached.complete);
        assert_eq!(cached.tracks[0].position, Some(2));
    }

    #[tokio::test]
    #[ignore = "uses the public iTunes and MusicBrainz catalogs"]
    async fn reads_real_artist_discography_and_complete_album_tracklist() {
        let client = client(false).unwrap();
        let (apple, brainz) = tokio::join!(
            apple_artist(&client, "CUPSIZE", &[]),
            brainz_artist(&client, "Portishead", &[])
        );
        assert!(!apple.unwrap().unwrap().releases.is_empty());
        let (_, groups) = brainz.unwrap().unwrap();
        assert!(groups
            .iter()
            .any(|release| release.kind == ReleaseKind::Album));
        let release = apple_release(
            &client,
            &ReleaseRef::new(Provider::Itunes, "6834158209".into()).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(release.complete);
        assert_eq!(release.tracks.len(), 1);
        assert!(confirms_track(
            &release,
            "Don Toliver",
            "Party With The Boys"
        ));
    }

    #[tokio::test]
    #[ignore = "uses the public iTunes and MusicBrainz catalogs"]
    async fn identifies_villian_and_loads_the_reported_releases() {
        let client = client(false).unwrap();
        let samples = [
            "Случайно Наступил На Шприц".to_owned(),
            "Общество Мертвых Поэтов".to_owned(),
        ];
        let (apple, brainz) = tokio::join!(
            apple_artist(&client, "VILLIAN", &samples),
            brainz_artist(&client, "Villian", &samples)
        );
        let apple = apple.unwrap().unwrap().releases;
        let (artist, brainz) = brainz.unwrap().unwrap();
        assert_eq!(artist.id, "d4ae5608-d7a0-46d1-b37a-f81138804b3c");
        let releases = combine_releases(apple, brainz);
        for (title, count) in [
            ("Красивое Зло", 7),
            ("Общество Мертвых Поэтов", 1),
            ("Случайно Наступил На Шприц", 1),
        ] {
            let release = releases.iter().find(|item| item.title == title).unwrap();
            if count == 1 {
                assert!(release.kind == ReleaseKind::Single);
            }
            let details = apple_release(&client, &release.reference)
                .await
                .unwrap()
                .unwrap();
            assert!(details.complete);
            assert_eq!(details.tracks.len(), count);
        }
    }
}
