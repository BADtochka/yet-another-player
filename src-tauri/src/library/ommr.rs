use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{Client, StatusCode, Url};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    cover,
    external_tools::ommr::{normalize_url, Configuration, LocalServer, Mode},
};
use tokio::{
    sync::Mutex,
    time::{sleep_until, Instant},
};

use super::{connect, queries, LibraryResult, Tags};

const PAGE_SIZE: i64 = 48;
const REQUEST_INTERVAL: Duration = Duration::from_millis(1200);

#[derive(Default)]
pub(super) struct Worker {
    revision: AtomicU64,
    next_request: Mutex<Option<Instant>>,
}

impl Worker {
    pub(super) fn schedule(
        self: &Arc<Self>,
        app: AppHandle,
        database: PathBuf,
        writer: Arc<Mutex<()>>,
        config: Configuration,
    ) {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let enabled = !matches!(config.mode, Mode::Off);
        let _ = app.emit("library:metadata-running", enabled);
        if !enabled {
            return;
        }
        let worker = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let result = async {
                if worker.revision.load(Ordering::Relaxed) != revision {
                    return Ok(());
                }
                let Some(resolver) = configured_resolver(&app, config).await? else {
                    return Ok(());
                };
                worker
                    .enrich(&app, &database, &writer, revision, &resolver, None)
                    .await
            }
            .await;
            if worker.revision.load(Ordering::Relaxed) == revision {
                let _ = app.emit("library:metadata-running", false);
                if let Err(error) = result {
                    eprintln!("OMMR: {error}");
                    let _ = app.emit("library:metadata-error", ());
                }
            }
        });
    }

    pub(super) async fn enrich_download(
        &self,
        app: &AppHandle,
        database: &Path,
        writer: &Mutex<()>,
        config: Configuration,
        id: i64,
    ) -> LibraryResult<()> {
        let Some(resolver) = configured_resolver(app, config).await? else {
            return Ok(());
        };
        let revision = self.revision.load(Ordering::Relaxed);
        self.enrich(app, database, writer, revision, &resolver, Some(id))
            .await
    }

    async fn enrich(
        &self,
        app: &AppHandle,
        database: &Path,
        writer: &Mutex<()>,
        revision: u64,
        resolver: &Resolver,
        download: Option<i64>,
    ) -> LibraryResult<()> {
        let current = || self.revision.load(Ordering::Relaxed) == revision;
        let mut cursor = 0;
        while current() {
            let rows = read_page(database, cursor, download)?;
            if rows.is_empty() {
                break;
            }
            let mut changed = Vec::new();
            let mut failure = None;
            for (id, path, original, source_video_id, downloaded_cover) in rows {
                cursor = id;
                if !current() {
                    publish(app, database, writer, changed).await?;
                    return Ok(());
                }
                let mut tags: Tags = serde_json::from_str(&original)?;
                let track_path = Path::new(&path);
                if !track_path.is_file()
                    || (!tags.missing_catalog_fields()
                        && (tags.cover_url.is_some()
                            || downloaded_cover
                            || cover::extract_cover(track_path).is_some()))
                {
                    continue;
                }
                let input = Lookup::new(&tags, track_path, source_video_id.as_deref());
                if input.reference.is_none()
                    && (input.title.is_empty()
                        || [
                            input.title.as_str(),
                            input.artist.as_str(),
                            input.album.as_str(),
                        ]
                        .iter()
                        .any(|field| field.len() > 1024 || field.contains(char::is_control)))
                {
                    continue;
                }
                let key =
                    serde_json::to_string(&(&resolver.cache_scope, &resolver.sources, &input))?;
                let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
                let mut next = self.next_request.lock().await;
                if !current() {
                    publish(app, database, writer, changed).await?;
                    return Ok(());
                }
                let cached = cached(&connect(database)?, &key, now)?;
                let track = match cached {
                    Some(track) => track,
                    None => match resolver.resolve(&input, &mut next, &current).await {
                        Ok(track) => {
                            if !current() {
                                publish(app, database, writer, changed).await?;
                                return Ok(());
                            }
                            let _writer = writer.lock().await;
                            connect(database)?.execute(
                                "INSERT OR REPLACE INTO resolver_cache VALUES (?1, ?2, ?3)",
                                params![key, now, serde_json::to_string(&track)?],
                            )?;
                            track
                        }
                        Err(error) => {
                            if !current() {
                                publish(app, database, writer, changed).await?;
                                return Ok(());
                            }
                            if error.downcast_ref::<reqwest::Error>().is_some_and(|error| {
                                error.status() == Some(StatusCode::BAD_REQUEST)
                            }) {
                                continue;
                            }
                            failure = Some(error);
                            break;
                        }
                    },
                };
                drop(next);
                if let Some(track) = track {
                    track.fill(&mut tags);
                    let metadata = serde_json::to_string(&tags)?;
                    if metadata != original {
                        changed.push((id, path, original, metadata, tags));
                    }
                }
                // Publish small groups so long scans show completed matches while requests continue.
                if changed.len() >= 8 {
                    publish(app, database, writer, std::mem::take(&mut changed)).await?;
                }
            }
            publish(app, database, writer, changed).await?;
            if let Some(error) = failure {
                return Err(error);
            }
            if download.is_some() {
                break;
            }
        }
        Ok(())
    }
}

async fn configured_resolver(
    app: &AppHandle,
    config: Configuration,
) -> LibraryResult<Option<Resolver>> {
    let (url, local) = match config.mode {
        Mode::Off => return Ok(None),
        Mode::Remote => (
            normalize_url(config.url.as_deref().unwrap_or_default())?,
            false,
        ),
        Mode::Local => {
            let Some(url) = app.state::<LocalServer>().endpoint(app).await? else {
                return Ok(None);
            };
            (url, true)
        }
    };
    Ok(Some(Resolver::new(&url, local, config.sources()?)?))
}

type CatalogRow = (i64, String, String, Option<String>, bool);
type ResolvedRow = (i64, String, String, String, Tags);

fn read_page(
    database: &Path,
    cursor: i64,
    download: Option<i64>,
) -> LibraryResult<Vec<CatalogRow>> {
    let db = connect(database)?;
    let mut statement = db.prepare(
        "SELECT t.id, t.path, t.metadata, y.video_id, y.cover IS NOT NULL FROM track t
         LEFT JOIN external_download y ON y.track_id=t.id
         WHERE t.id>?1 AND EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
           AND (?3 IS NULL OR (t.id=?3 AND y.video_id IS NOT NULL))
         ORDER BY t.id LIMIT ?2",
    )?;
    let rows = statement.query_map(params![cursor, PAGE_SIZE, download], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn cached(db: &Connection, key: &str, now: i64) -> LibraryResult<Option<Option<Track>>> {
    let response: Option<String> = db
        .query_row(
            "SELECT response FROM resolver_cache WHERE query=?1
         AND checked >= ?2 - CASE WHEN response='null' THEN 86400 ELSE 2592000 END",
            params![key, now],
            |row| row.get(0),
        )
        .optional()?;
    Ok(response.and_then(|value| serde_json::from_str(&value).ok()))
}

async fn publish(
    app: &AppHandle,
    database: &Path,
    writer: &Mutex<()>,
    rows: Vec<ResolvedRow>,
) -> LibraryResult<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let _writer = writer.lock().await;
    let mut db = connect(database)?;
    let tx = db.transaction()?;
    let mut updated = false;
    for (id, path, original, metadata, tags) in rows {
        if tx.execute(
            "UPDATE track SET metadata=?2 WHERE id=?1 AND metadata=?3
             AND EXISTS (SELECT 1 FROM source WHERE track_id=track.id)",
            params![id, metadata, original],
        )? > 0
        {
            queries::index(&tx, id, &path, &tags)?;
            updated = true;
        }
    }
    if updated {
        super::versions::rebuild(&tx)?;
        tx.execute("UPDATE catalog_state SET revision=revision+1", [])?;
    }
    tx.commit()?;
    if updated {
        let _ = app.emit("library:metadata", ());
    }
    Ok(())
}

#[derive(Serialize)]
struct Lookup {
    title: String,
    artist: String,
    album: String,
    reference: Option<(&'static str, String)>,
    duration: Option<f64>,
}

impl Lookup {
    fn new(tags: &Tags, path: &Path, source_video_id: Option<&str>) -> Self {
        let filename = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let guessed = filename.split_once(" - ").filter(|(artist, _)| {
            !artist.trim().is_empty() && !artist.trim().chars().all(|c| c.is_ascii_digit())
        });
        let artist = tags
            .artists
            .first()
            .map(String::as_str)
            .or_else(|| {
                tags.album_artist
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
            })
            .or_else(|| guessed.map(|(artist, _)| artist))
            .unwrap_or("");
        let title = if tags.fallback_title || tags.name == filename {
            guessed.map_or(tags.name.as_str(), |(_, title)| title)
        } else {
            &tags.name
        };
        let reference = tags
            .isrc
            .as_deref()
            .filter(|isrc| !isrc.trim().is_empty())
            .map(|isrc| ("isrc", isrc.trim().to_owned()))
            .or_else(|| source_video_id.map(|id| (concat!("you", "tube_id"), id.to_owned())));
        Self {
            title: title.trim().to_owned(),
            artist: artist.trim().to_owned(),
            album: tags.album.as_deref().unwrap_or("").trim().to_owned(),
            reference,
            duration: tags.duration,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Track {
    title: String,
    artists: Option<Vec<Artist>>,
    album: Option<Album>,
    release_date: String,
    duration_ms: Option<u64>,
    disc_number: Option<u32>,
    track_number: Option<u32>,
    genres: Option<Vec<String>>,
    images: Option<Vec<Artwork>>,
    isrc: String,
    provider_data: Option<ProviderData>,
}

#[derive(Deserialize, Serialize)]
struct Artist {
    name: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Album {
    title: String,
    release_date: String,
    images: Option<Vec<Artwork>>,
}

#[derive(Deserialize, Serialize)]
struct Artwork {
    url: String,
}

#[derive(Deserialize, Serialize)]
struct ProviderData {
    deezer: Option<Deezer>,
}

#[derive(Deserialize, Serialize)]
struct Deezer {
    catalog: Option<DeezerCatalog>,
}

#[derive(Deserialize, Serialize)]
struct DeezerCatalog {
    album: Option<DeezerAlbum>,
}

#[derive(Deserialize, Serialize)]
struct DeezerAlbum {
    title: Option<String>,
    artist: Option<Artist>,
}

#[derive(Deserialize)]
struct Resolution {
    track: Option<Track>,
}

#[derive(Deserialize)]
struct Search {
    results: Option<Vec<SearchResult>>,
}

#[derive(Deserialize)]
struct SearchResult {
    title: String,
    artists: Option<Vec<Artist>>,
}

fn clean(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn fill(field: &mut Option<String>, value: Option<String>) {
    if field.as_deref().is_none_or(|value| value.trim().is_empty()) && value.is_some() {
        *field = value;
    }
}

impl Tags {
    fn missing_catalog_fields(&self) -> bool {
        self.fallback_title
            || self.name.trim().is_empty()
            || self.artists.is_empty()
            || self.year.is_none()
            || self.disc_number.is_none()
            || self.track_number.is_none()
            || self.duration.is_none()
            || [
                &self.album,
                &self.album_artist,
                &self.release_date,
                &self.genre,
                &self.isrc,
            ]
            .iter()
            .any(|field| field.as_deref().is_none_or(|value| value.trim().is_empty()))
    }
}

impl Track {
    fn matches_duration(&self, duration: Option<f64>) -> bool {
        duration
            .zip(self.duration_ms)
            .is_some_and(|(local, remote)| {
                local > 0.0 && (local - remote as f64 / 1000.0).abs() <= 3.0
            })
    }

    fn fill(self, tags: &mut Tags) {
        let album = self.album.unwrap_or_default();
        if (tags.fallback_title || tags.name.trim().is_empty()) && !self.title.trim().is_empty() {
            tags.name = self.title.trim().to_owned();
            tags.fallback_title = false;
        }
        let artists = self
            .artists
            .unwrap_or_default()
            .into_iter()
            .filter_map(|artist| clean(&artist.name))
            .collect::<Vec<_>>();
        if tags.artists.is_empty() {
            tags.artists = artists;
        }
        let matching_edition = tags.album.as_deref().is_none_or(|local| {
            local.trim().is_empty()
                || local.trim().to_lowercase() == album.title.trim().to_lowercase()
        });
        fill(&mut tags.album, clean(&album.title));
        if matching_edition {
            let album_artist = self
                .provider_data
                .and_then(|data| data.deezer)
                .and_then(|data| data.catalog)
                .and_then(|catalog| catalog.album)
                .filter(|native| {
                    native
                        .title
                        .as_deref()
                        .is_some_and(|title| title == album.title)
                })
                .and_then(|native| native.artist)
                .and_then(|artist| clean(&artist.name));
            fill(&mut tags.album_artist, album_artist);
            let date = clean(&album.release_date).or_else(|| clean(&self.release_date));
            let year = date
                .as_deref()
                .and_then(|date| date.get(..4)?.parse::<u16>().ok());
            if tags.year.is_none() || tags.year == year {
                fill(&mut tags.release_date, date);
                tags.year = tags.year.or(year);
            }
            tags.disc_number = tags
                .disc_number
                .or(self.disc_number.filter(|number| *number > 0));
            tags.track_number = tags
                .track_number
                .or(self.track_number.filter(|number| *number > 0));
        }
        tags.duration = tags.duration.or(self
            .duration_ms
            .filter(|ms| *ms > 0)
            .map(|ms| ms as f64 / 1000.0));
        fill(&mut tags.isrc, clean(&self.isrc));
        fill(
            &mut tags.genre,
            self.genres
                .unwrap_or_default()
                .iter()
                .find_map(|genre| clean(genre)),
        );
        let images = album.images.into_iter().chain(self.images).flatten();
        let cover = images
            .filter_map(|image| Url::parse(&image.url).ok())
            .find(|url| {
                url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
            })
            .map(String::from);
        if matching_edition {
            fill(&mut tags.cover_url, cover);
        }
    }
}

struct Resolver {
    base: String,
    cache_scope: String,
    sources: String,
    client: Client,
}

fn resolver_client(local: bool) -> reqwest::Result<Client> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(45));
    if local {
        client.no_proxy().build()
    } else {
        client.build()
    }
}

impl Resolver {
    fn new(url: &str, local: bool, sources: String) -> LibraryResult<Self> {
        let base = normalize_url(url)?;
        Ok(Self {
            cache_scope: if local { "local".into() } else { base.clone() },
            base,
            sources,
            client: resolver_client(local)?,
        })
    }

    async fn resolve(
        &self,
        input: &Lookup,
        next: &mut Option<Instant>,
        current: &(impl Fn() -> bool + Sync),
    ) -> LibraryResult<Option<Track>> {
        if let Some((key, value)) = &input.reference {
            let result: Resolution = self
                .get("resolve", &[(*key, value.as_str())], next, current)
                .await?;
            return Ok(result.track);
        }
        let inferred_artist = input.artist.is_empty();
        let mut artist = input.artist.clone();
        if inferred_artist {
            if input.duration.is_none() {
                return Ok(None);
            }
            let response: Search = self
                .get(
                    "search",
                    &[("q", &input.title), ("limit", "8")],
                    next,
                    current,
                )
                .await?;
            let mut names = response
                .results
                .unwrap_or_default()
                .into_iter()
                .filter(|track| track.title.trim().to_lowercase() == input.title.to_lowercase())
                .filter_map(|track| track.artists?.into_iter().next())
                .map(|artist| artist.name)
                .collect::<Vec<_>>();
            names.sort_by_cached_key(|name| name.to_lowercase());
            names.dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
            if names.len() != 1 {
                return Ok(None);
            }
            artist = names.remove(0);
        }
        let mut query = vec![("artist", artist.as_str()), ("title", input.title.as_str())];
        if !input.album.is_empty() {
            query.push(("album", &input.album));
        }
        let result: Resolution = self.get("resolve", &query, next, current).await?;
        Ok(result.track.filter(|track| {
            track.matches_duration(input.duration)
                || (!inferred_artist
                    && (track.duration_ms.is_none_or(|ms| ms == 0) || input.duration.is_none()))
        }))
    }

    async fn get<T: DeserializeOwned>(
        &self,
        route: &str,
        query: &[(&str, &str)],
        next: &mut Option<Instant>,
        current: &(impl Fn() -> bool + Sync),
    ) -> LibraryResult<T> {
        const LIMIT: usize = 2 * 1024 * 1024;
        let sources = (!self.sources.is_empty()).then_some(("sources", self.sources.as_str()));
        let sources = sources.as_slice();
        let mut backoff = 3;
        loop {
            while let Some(time) = next.filter(|time| *time > Instant::now()) {
                if !current() {
                    return Err("OMMR scan replaced".into());
                }
                sleep_until(time.min(Instant::now() + Duration::from_secs(1))).await;
            }
            if !current() {
                return Err("OMMR scan replaced".into());
            }
            let response = self
                .client
                .get(format!("{}/v1/{route}", self.base))
                .query(query)
                .query(&sources)
                .send()
                .await;
            *next = Some(Instant::now() + REQUEST_INTERVAL);
            let mut response = response?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS
                || response.status() == StatusCode::SERVICE_UNAVAILABLE
            {
                let retry = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(backoff)
                    .clamp(2, 86_400);
                *next = Some(Instant::now() + Duration::from_secs(retry));
                backoff = (backoff * 2).min(120);
                continue;
            }
            response = response.error_for_status()?;
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if body.len() + chunk.len() > LIMIT {
                    return Err("OMMR response too large".into());
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(serde_json::from_slice(&body)?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retries_throttled_requests_using_retry_after() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            thread,
        };

        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.local_addr().unwrap());
        let responder = thread::spawn(move || {
            let mut last_request = None;
            for response in [
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 2\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{\"track\":null}",
            ] {
                let (mut socket, _) = server.accept().unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert!(request.starts_with("GET /v1/resolve?isrc=TEST00000001 "));
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                        break;
                    }
                }
                let now = Instant::now();
                if let Some(previous) = last_request {
                    assert!(now - previous >= Duration::from_secs(2));
                }
                last_request = Some(now);
                socket.write_all(response.as_bytes()).unwrap();
            }
        });

        let resolver = Resolver::new(&url, false, String::new()).unwrap();
        let tags = Tags {
            isrc: Some("TEST00000001".into()),
            ..Tags::default()
        };
        let input = Lookup::new(&tags, Path::new("/music/song.opus"), None);
        let mut next = None;
        let result = resolver.resolve(&input, &mut next, &|| true).await.unwrap();
        responder.join().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn production_search_projection_and_resolve_preserve_local_tags() {
        let search: Search = serde_json::from_str(
            r#"{"results":[{"title":"Песня","artists":[{"name":"Артист"}],"album":"Релиз"}]}"#,
        )
        .unwrap();
        assert_eq!(search.results.unwrap()[0].title, "Песня");

        let mut tags = Tags {
            name: "Артист - Песня".into(),
            fallback_title: true,
            genre: Some("Локальный жанр".into()),
            ..Tags::default()
        };
        let input = Lookup::new(&tags, Path::new("/music/Артист - Песня.opus"), None);
        assert_eq!(
            (input.artist.as_str(), input.title.as_str()),
            ("Артист", "Песня")
        );

        let resolved: Resolution = serde_json::from_str(r#"{"track":{"title":"Песня","artists":[{"name":"Артист"}],"album":{"title":"Релиз","release_date":"2026-02-17"},"genres":["Поп"],"track_number":3}}"#).unwrap();
        resolved.track.unwrap().fill(&mut tags);
        assert_eq!(tags.name, "Песня");
        assert_eq!(tags.album.as_deref(), Some("Релиз"));
        assert_eq!(tags.release_date.as_deref(), Some("2026-02-17"));
        assert_eq!(tags.genre.as_deref(), Some("Локальный жанр"));
    }
}
