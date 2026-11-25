pub(crate) mod catalog;
pub(crate) mod edits;
pub(crate) mod external;
mod ommr;
pub mod playlists;
pub mod queries;
pub(crate) mod trash;
pub(crate) mod versions;

use std::{
    collections::{HashMap, HashSet},
    fs,
    panic::catch_unwind,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

use lofty::{
    config::ParseOptions,
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::{Accessor, ItemKey},
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, State};
use walkdir::WalkDir;

use crate::{audio, MusicFolders};

type LibraryError = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type LibraryResult<T> = Result<T, LibraryError>;
const CATALOG_VERSION: u32 = 19;

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Tags {
    name: String,
    artists: Vec<String>,
    lyrics: Vec<String>,
    album: Option<String>,
    album_artist: Option<String>,
    year: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    is_single: Option<bool>,
    #[serde(default)]
    is_ep: bool,
    duration: Option<f64>,
    replay_gain_db: Option<f32>,
    disc_number: Option<u32>,
    track_number: Option<u32>,
    genre: Option<String>,
    #[serde(default)]
    genres: Vec<String>,
    release_date: Option<String>,
    isrc: Option<String>,
    cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_video_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clip_video_id: Option<String>,
    #[serde(skip_serializing)]
    catalog_album_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    has_local_video: bool,
    #[serde(default)]
    fallback_title: bool,
    #[serde(default)]
    lyrics_offset_ms: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clip_lyrics_offset_ms: Option<i32>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) struct PlaybackTags {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Serialize)]
pub struct Track {
    id: i64,
    path: String,
    #[serde(rename = "albumId")]
    album_id: String,
    #[serde(rename = "playCount")]
    play_count: i64,
    #[serde(flatten)]
    tags: Tags,
}

impl Track {
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    fn from_metadata(
        id: i64,
        path: String,
        metadata: &str,
        play_count: i64,
    ) -> LibraryResult<Self> {
        let tags: Tags = serde_json::from_str(metadata)?;
        let album_id = match &tags.catalog_album_id {
            Some(value) => value.clone(),
            None => tags.album_id(id)?,
        };
        Ok(Self {
            id,
            path,
            album_id,
            play_count,
            tags,
        })
    }
}

impl Tags {
    fn album_name(&self) -> Option<&str> {
        self.album
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    fn album_id(&self, id: i64) -> LibraryResult<String> {
        match self.album_name() {
            Some(album) => {
                let artists = self.artists.join(", ");
                let artist = self
                    .album_artist
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&artists);
                Ok(serde_json::to_string(&[album, artist])?)
            }
            None => Ok(format!("single:{id}")),
        }
    }
}

#[cfg(test)]
struct Catalog {
    tracks: Vec<Track>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStatus {
    initialized: bool,
    track_count: i64,
}

#[derive(Serialize)]
pub struct Progress {
    current: usize,
    total: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    cancelled: bool,
    unavailable_folders: usize,
    catalog: CatalogStatus,
    read_warnings: ReadWarnings,
}

#[derive(Default, Serialize)]
struct ReadWarnings {
    count: usize,
    example: Option<String>,
}

impl ReadWarnings {
    fn record(&mut self, path: &Path, stage: &str, detail: &str) {
        self.count += 1;
        self.example.get_or_insert_with(|| {
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("?");
            format!("{stage} (.{extension}): {detail}")
        });
    }
}

struct Job {
    id: String,
    cancelled: Arc<AtomicBool>,
}

type ActiveJob = Arc<Mutex<Option<Job>>>;

pub struct Library {
    database: PathBuf,
    active: ActiveJob,
    catalog_writer: Arc<tokio::sync::Mutex<()>>,
    resolver: Arc<ommr::Worker>,
}

impl Library {
    pub fn new(database: PathBuf) -> LibraryResult<Self> {
        if let Some(parent) = database.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut connection = connect(&database)?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > CATALOG_VERSION {
            return Err(format!(
                "Unsupported catalog version {version} (maximum {CATALOG_VERSION})"
            )
            .into());
        }
        if (4..9).contains(&version) {
            external::migrate_source_names(&mut connection)?;
        }
        if version == 0 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "CREATE TABLE track (
                    id INTEGER PRIMARY KEY,
                    path TEXT UNIQUE NOT NULL,
                    stamp TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    metadata TEXT NOT NULL
                );
                CREATE TABLE source (
                    root TEXT NOT NULL,
                    track_id INTEGER NOT NULL REFERENCES track(id),
                    PRIMARY KEY (root, track_id)
                );
                CREATE INDEX source_track ON source(track_id);
                CREATE TABLE catalog_state (initialized INTEGER NOT NULL);
                INSERT INTO catalog_state VALUES (0);
                PRAGMA user_version = 1;",
            )?;
            tx.commit()?;
        }
        if version < 2 {
            playlists::migrate(&mut connection)?;
        }
        if version < 3 {
            queries::migrate(&mut connection)?;
        }
        if version < 4 {
            external::migrate(&mut connection)?;
        }
        if version < 5 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "CREATE TABLE fetched_lyrics (
                    track_id INTEGER PRIMARY KEY REFERENCES track(id),
                    text TEXT NOT NULL
                );
                CREATE TABLE resolver_cache (
                    query TEXT PRIMARY KEY, checked INTEGER NOT NULL, response TEXT NOT NULL
                );
                PRAGMA user_version = 5;",
            )?;
            tx.commit()?;
        }
        if version < 6 {
            queries::migrate_added_date(&mut connection)?;
        }
        if version < 7 {
            queries::migrate_singles(&mut connection)?;
        }
        if version < 8 {
            versions::migrate(&mut connection)?;
        }
        if version < 9 {
            versions::refresh_local_video(&mut connection)?;
        }
        if version < 10 {
            migrate_play_counts(&mut connection)?;
        }
        if version < 11 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "CREATE TABLE playback_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    track_id INTEGER NOT NULL REFERENCES track(id),
                    played_at INTEGER NOT NULL
                 );
                 CREATE TABLE metadata_log (
                    track_id INTEGER PRIMARY KEY REFERENCES track(id),
                    status TEXT NOT NULL
                 );
                 PRAGMA user_version=11;",
            )?;
            tx.commit()?;
        }
        if version < 12 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "ALTER TABLE metadata_log ADD COLUMN attempts TEXT NOT NULL DEFAULT '[]';
                              PRAGMA user_version=12;",
            )?;
            tx.commit()?;
        }
        if version < 13 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "CREATE TABLE track_edits (track_id INTEGER PRIMARY KEY REFERENCES track(id),
                    fields TEXT NOT NULL, offset_ms INTEGER NOT NULL);
                 CREATE TABLE track_trash (track_id INTEGER PRIMARY KEY REFERENCES track(id),
                    removed_at INTEGER NOT NULL, directory TEXT NOT NULL, roots TEXT NOT NULL);
                 CREATE TABLE trash_settings (retention_days INTEGER NOT NULL);
                 INSERT INTO trash_settings VALUES (30);
                 PRAGMA user_version=13;",
            )?;
            tx.commit()?;
        }
        if version < 14 {
            trash::migrate_entries(&mut connection)?;
        }
        if version < 15 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "CREATE TABLE resolver_artwork (url TEXT PRIMARY KEY, bytes BLOB NOT NULL);
                 CREATE TABLE release_artwork (album_id TEXT PRIMARY KEY, url TEXT NOT NULL REFERENCES resolver_artwork(url));
                 PRAGMA user_version=15;",
            )?;
            tx.commit()?;
        }
        if version < 16 {
            let tx = connection.transaction()?;
            versions::rebuild(&tx)?;
            tx.execute_batch(
                "UPDATE catalog_state SET revision=revision+1; PRAGMA user_version=16;",
            )?;
            tx.commit()?;
        }
        if version < 17 {
            playlists::migrate_transitions(&mut connection)?;
        }
        if version < 18 {
            catalog::storage::migrate(&mut connection)?;
        }
        Ok(Self {
            database,
            active: Arc::default(),
            catalog_writer: Arc::default(),
            resolver: Arc::default(),
        })
    }

    pub(crate) fn database_path(&self) -> &Path {
        &self.database
    }

    pub(crate) fn local_video(&self, audio: &Path) -> LibraryResult<Option<String>> {
        Ok(versions::video_path(&connect(&self.database)?, audio)?)
    }

    pub(crate) fn owns_clip(&self, audio: &Path, video_id: &str) -> LibraryResult<bool> {
        Ok(connect(&self.database)?.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM track t WHERE t.path=?1 AND (
                 json_extract(t.metadata, '$.externalVideoId')=?2
                 OR json_extract(t.metadata, '$.clipVideoId')=?2
                 OR EXISTS (SELECT 1 FROM track_variant r WHERE r.audio_id=t.id AND r.clip_id=?2)
               )
             )",
            rusqlite::params![audio.to_str(), video_id],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn record_play(&self, path: &str) -> rusqlite::Result<Option<String>> {
        let _writer = self.catalog_writer.blocking_lock();
        let db = connect(&self.database)?;
        db.query_row(
            "UPDATE track SET play_count=play_count+1
             WHERE id=(SELECT COALESCE(v.audio_id, t.id) FROM track t
                       LEFT JOIN track_variant v ON v.variant_id=t.id WHERE t.path=?1)
             RETURNING path",
            [path],
            |row| row.get(0),
        )
        .optional()
    }

    pub(crate) fn record_playback_start(&self, path: &str) -> rusqlite::Result<bool> {
        let _writer = self.catalog_writer.blocking_lock();
        let db = connect(&self.database)?;
        Ok(db.execute(
            "INSERT INTO playback_history (track_id, played_at)
             SELECT COALESCE(v.audio_id, t.id), CAST(unixepoch() AS INTEGER) FROM track t
             LEFT JOIN track_variant v ON v.variant_id=t.id WHERE t.path=?1",
            [path],
        )? != 0)
    }

    pub(crate) fn fetched_lyrics(&self, path: &Path) -> Option<String> {
        read_fetched_lyrics(&self.database, path).ok().flatten()
    }

    pub(crate) fn resolver_cover(&self, path: &Path) -> Option<Vec<u8>> {
        connect(&self.database)
            .ok()?
            .query_row(
                "SELECT art.bytes FROM track t
                 JOIN resolver_artwork art ON art.url=json_extract(t.metadata, '$.coverUrl')
                 WHERE t.path=?1",
                [path.to_str()?],
                |row| row.get(0),
            )
            .ok()
    }

    pub(crate) fn release_cover(&self, path: &Path) -> Option<Vec<u8>> {
        connect(&self.database)
            .ok()?
            .query_row(
                "SELECT art.bytes FROM track t
                 LEFT JOIN track_variant v ON v.variant_id=t.id
                 JOIN track_index i ON i.id=COALESCE(v.audio_id, t.id)
                 JOIN release_artwork release ON release.album_id=i.album_id
                 JOIN resolver_artwork art ON art.url=release.url WHERE t.path=?1",
                [path.to_str()?],
                |row| row.get(0),
            )
            .ok()
    }

    pub(crate) async fn lock_catalog_writer(&self) -> tokio::sync::OwnedMutexGuard<()> {
        Arc::clone(&self.catalog_writer).lock_owned().await
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    pub(crate) fn playback_tags(&self, path: &Path) -> PlaybackTags {
        let tags = (|| {
            let db = connect(&self.database).ok()?;
            let metadata: String = db
                .query_row(
                    "SELECT metadata FROM track WHERE path = ?1",
                    [path.to_str()?],
                    |row| row.get(0),
                )
                .ok()?;
            serde_json::from_str(&metadata).ok()
        })()
        .unwrap_or_else(|| read_tags(path, &mut ReadWarnings::default()));
        PlaybackTags {
            title: tags.name,
            artists: tags.artists,
            album: tags.album,
            duration: tags.duration,
        }
    }

    pub(crate) fn index_download(
        &self,
        path: &Path,
        roots: &[String],
        download: external::Download<'_>,
    ) -> LibraryResult<Track> {
        let sources = roots
            .iter()
            .filter(|root| fs::canonicalize(root).is_ok_and(|root| path.starts_with(root)))
            .collect::<Vec<_>>();
        if sources.is_empty() || !supported(path) {
            return Err(
                "Downloaded file is outside the library or has an unsupported format".into(),
            );
        }
        let mut db = connect(&self.database)?;
        let tx = db.transaction()?;
        let id = index_track(
            &tx,
            path,
            Some((download.video_id, download.tags)),
            &mut ReadWarnings::default(),
        )?;
        external::record(&tx, id, download)?;
        mark_catalog_addition(&tx, id)?;
        for root in sources {
            tx.execute(
                "INSERT OR IGNORE INTO source VALUES (?1, ?2)",
                params![root, id],
            )?;
        }
        versions::rebuild(&tx)?;
        let id = versions::audio_id(&tx, id)?;
        let (path, tags, play_count) = tx.query_row(
            &format!(
                "SELECT t.path, {}, {} FROM track t WHERE t.id=?1",
                versions::METADATA,
                versions::PLAY_COUNT
            ),
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get(2)?,
                ))
            },
        )?;
        let track = Track::from_metadata(id, path, &tags, play_count)?;
        tx.execute("UPDATE catalog_state SET revision = revision + 1", [])?;
        tx.commit()?;
        Ok(track)
    }
}

fn connect(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "foreign_keys", true)?;
    Ok(db)
}

fn migrate_play_counts(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "ALTER TABLE track ADD COLUMN play_count INTEGER NOT NULL DEFAULT 0;
         PRAGMA user_version=10;",
    )?;
    tx.commit()
}

pub(crate) fn read_fetched_lyrics(database: &Path, path: &Path) -> LibraryResult<Option<String>> {
    Ok(connect(database)?
        .query_row(
            "SELECT l.text FROM fetched_lyrics l JOIN track t ON t.id=l.track_id WHERE t.path=?1",
            [path.to_str()],
            |row| row.get(0),
        )
        .optional()?)
}

#[derive(Default)]
pub(crate) struct TransitionLyrics {
    pub(crate) embedded: Vec<String>,
    pub(crate) fetched: Option<String>,
    pub(crate) offset_ms: i32,
}

pub(crate) fn read_transition_lyrics(database: &Path, path: &Path) -> LibraryResult<TransitionLyrics> {
    let row = connect(database)?
        .query_row(
            "SELECT json_extract(t.metadata, '$.lyrics'), l.text,
                COALESCE(json_extract(t.metadata, '$.lyricsOffsetMs'), 0)
             FROM track t LEFT JOIN fetched_lyrics l ON l.track_id=t.id WHERE t.path=?1",
            [path.to_str()],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(row.map_or_else(
        TransitionLyrics::default,
        |(embedded, fetched, offset_ms)| TransitionLyrics {
            embedded: embedded
                .as_deref()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default(),
            fetched,
            offset_ms,
        },
    ))
}

pub(crate) fn save_fetched_lyrics(database: &Path, path: &Path, text: &str) -> LibraryResult<()> {
    let db = connect(database)?;
    if db.execute(
        "INSERT INTO fetched_lyrics (track_id, text)
         SELECT t.id, ?2 FROM track t WHERE t.path=?1
           AND EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
         ON CONFLICT(track_id) DO UPDATE SET text=excluded.text",
        params![path.to_str(), text],
    )? == 0
    {
        return Err("Track is absent from the catalog".into());
    }
    Ok(())
}

fn read_status(db: &Connection) -> LibraryResult<CatalogStatus> {
    Ok(db.query_row(
        "SELECT initialized, (SELECT COUNT(DISTINCT track_id) FROM source WHERE track_id NOT IN (SELECT variant_id FROM track_variant)) FROM catalog_state",
        [],
        |row| {
            Ok(CatalogStatus {
                initialized: row.get(0)?,
                track_count: row.get(1)?,
            })
        },
    )?)
}

#[cfg(test)]
fn read_catalog(db: &Connection) -> LibraryResult<Catalog> {
    let mut query = db.prepare(
        "SELECT id, path, metadata, play_count FROM track
         WHERE EXISTS (SELECT 1 FROM source WHERE track_id = track.id)
         ORDER BY path",
    )?;
    let mut rows = query.query(())?;
    let mut tracks = Vec::new();
    while let Some(row) = rows.next()? {
        tracks.push(Track::from_metadata(
            row.get(0)?,
            row.get(1)?,
            &row.get::<_, String>(2)?,
            row.get(3)?,
        )?);
    }
    Ok(Catalog { tracks })
}

pub(crate) fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp3" | "m4a" | "mp4" | "ogg" | "opus" | "webm" | "wav" | "flac"
            )
        })
}

fn read_tags(path: &Path, warnings: &mut ReadWarnings) -> Tags {
    let mut tags = Tags {
        name: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        fallback_title: true,
        ..Tags::default()
    };
    let mut options = ParseOptions::new().read_cover_art(false);
    let probe = |options| {
        catch_unwind(|| Probe::open(path).and_then(|probe| probe.options(options).read()))
    };
    let file = match probe(options) {
        Ok(file) => file.ok(),
        Err(panic) => {
            warnings.record(path, "Lofty metadata", crate::panic_message(&*panic));
            // Lofty can fail while calculating stream properties even when the tags are readable.
            probe(options.read_properties(false))
                .ok()
                .and_then(Result::ok)
        }
    };
    if let Some(file) = file {
        let duration = file.properties().duration().as_secs_f64();
        tags.duration = (duration > 0.0).then_some(duration);
        if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
            if let Some(title) = tag.title().filter(|title| !title.trim().is_empty()) {
                tags.name = title.into_owned();
                tags.fallback_title = false;
            }
            tags.artists = tag
                .get_strings(ItemKey::TrackArtist)
                .flat_map(|name| name.split(';'))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect();
            tags.album = tag.album().map(|name| name.into_owned());
            tags.album_artist = tag.get_string(ItemKey::AlbumArtist).map(str::to_owned);
            let release_type = tag
                .get_strings(ItemKey::MusicBrainzReleaseType)
                .flat_map(|kind| kind.split(';'))
                .find_map(|kind| match kind.trim().to_ascii_lowercase().as_str() {
                    "single" => Some(catalog::ReleaseKind::Single),
                    "ep" => Some(catalog::ReleaseKind::Ep),
                    "album" => Some(catalog::ReleaseKind::Album),
                    _ => None,
                });
            tags.is_single = release_type.map(|kind| kind == catalog::ReleaseKind::Single);
            tags.is_ep = release_type == Some(catalog::ReleaseKind::Ep);
            let date = tag.date();
            let release: Option<lofty::tag::items::Timestamp> = tag
                .get_string(ItemKey::ReleaseDate)
                .and_then(|value| value.parse().ok());
            let release = match (release, date) {
                (Some(release), Some(date))
                    if release.year == date.year && release.month.is_none() =>
                {
                    Some(date)
                }
                (release, date) => release.or(date),
            };
            tags.year = release.map(|date| date.year);
            tags.release_date = release.map(|date| date.to_string());
            tags.isrc = tag.get_string(ItemKey::Isrc).map(str::to_owned);
            tags.track_number = tag.track();
            tags.disc_number = tag.disk();
            tags.genre = tag.genre().map(|genre| genre.into_owned());
            tags.genres = tag.get_strings(ItemKey::Genre).map(str::to_owned).collect();
            tags.replay_gain_db = tag
                .get_string(ItemKey::ReplayGainTrackGain)
                .and_then(|gain| gain.split_whitespace().next()?.parse::<f32>().ok())
                .filter(|gain| gain.is_finite());
            tags.lyrics = tag
                .get_string(ItemKey::UnsyncLyrics)
                .or_else(|| tag.get_string(ItemKey::Lyrics))
                .map(|text| text.lines().map(str::to_owned).collect())
                .unwrap_or_default();
        }
    }
    if tags.duration.is_none() {
        match audio::source::open(path) {
            Ok(source) => {
                tags.duration = source
                    .total_duration()
                    .map(|duration| duration.as_secs_f64())
            }
            Err(error) => {
                if let Some(error) = error
                    .downcast_ref::<std::io::Error>()
                    .filter(|error| error.kind() == std::io::ErrorKind::InvalidData)
                {
                    warnings.record(path, "Track duration", &error.to_string());
                }
            }
        }
    }
    tags
}

fn index_track(
    tx: &Transaction<'_>,
    path: &Path,
    downloaded_tags: Option<(&str, &external::DownloadTags)>,
    warnings: &mut ReadWarnings,
) -> LibraryResult<i64> {
    let metadata = fs::metadata(path)?;
    let stamp = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_nanos().to_string())
        .unwrap_or_default();
    let key = path.to_str().ok_or("File path is not valid UTF-8")?;
    let size = i64::try_from(metadata.len())?;
    if downloaded_tags.is_none() && !stamp.is_empty() {
        let existing = tx
            .query_row(
                "SELECT id FROM track WHERE path = ?1 AND stamp = ?2 AND size = ?3
                 AND json_type(metadata, '$.genres') IS NOT NULL
                 AND json_type(metadata, '$.isEp') IS NOT NULL",
                params![key, stamp, size],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
    }
    let mut tags = read_tags(path, warnings);
    tags.has_local_video = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        && versions::has_video(path);
    if let Some((video_id, downloaded)) = downloaded_tags {
        downloaded.apply(&mut tags);
        if downloaded.is_clip {
            tags.clip_video_id = Some(video_id.to_owned());
        }
        tags.external_video_id = Some(video_id.to_owned());
    } else if let Some((video_id, downloaded)) = external::stored_tags(tx, key)? {
        downloaded.apply(&mut tags);
        if downloaded.is_clip {
            tags.clip_video_id = Some(video_id.to_owned());
        }
        tags.external_video_id = Some(video_id);
    } else {
        tags.external_video_id = external::legacy_id(path).map(str::to_owned);
    }
    edits::apply(tx, key, &mut tags)?;
    let id = tx.query_row(
        "INSERT INTO track (path, stamp, size, metadata)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET stamp=excluded.stamp, size=excluded.size, metadata=excluded.metadata
         RETURNING id",
        params![key, stamp, size, serde_json::to_string(&tags)?],
        |row| row.get(0),
    )?;
    queries::index(tx, id, key, &tags)?;
    Ok(id)
}

fn mark_catalog_addition(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<()> {
    tx.prepare_cached(
        "UPDATE track
         SET added_at=MAX(CAST(unixepoch('subsec') * 1000 AS INTEGER),
             COALESCE((SELECT COALESCE(added_at,0) FROM track
                       ORDER BY -COALESCE(added_at,0), -id LIMIT 1), 0) + 1)
         WHERE id=?1 AND NOT EXISTS (SELECT 1 FROM source WHERE track_id=?1)",
    )?
    .execute([id])?;
    Ok(())
}

fn infer_scanned_albums(tx: &Transaction<'_>, cancelled: &AtomicBool) -> LibraryResult<()> {
    let first_artist = "json_extract(t.metadata, '$.artists[0]')";
    let mut known = tx.prepare(&format!(
        "SELECT t.path, {first_artist}, i.album_name, i.album_artist
         FROM track t JOIN track_index i ON i.id=t.id
         WHERE {first_artist} IS NOT NULL AND NULLIF(TRIM(json_extract(t.metadata, '$.album')), '') IS NOT NULL
           AND EXISTS (SELECT 1 FROM scan_sources WHERE track_id=t.id)",
    ))?;
    let mut rows = known.query([])?;
    let mut albums = HashMap::<_, Option<(String, String)>>::new();
    while let Some(row) = rows.next()? {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let path: String = row.get(0)?;
        let artist: String = row.get(1)?;
        let album: String = row.get(2)?;
        let album_artist: String = row.get(3)?;
        let Some(folder) = Path::new(&path).parent() else {
            continue;
        };
        if !folder
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.trim().to_lowercase() == album.trim().to_lowercase())
        {
            continue;
        }
        let group = (album, album_artist);
        albums
            .entry((folder.to_owned(), artist.trim().to_lowercase()))
            .and_modify(|existing| {
                if existing.as_ref() != Some(&group) {
                    *existing = None;
                }
            })
            .or_insert_with(|| Some(group));
    }
    if albums.is_empty() {
        return Ok(());
    }
    let mut missing = tx.prepare(&format!(
        "SELECT t.id, t.path, {first_artist}
         FROM track t WHERE {first_artist} IS NOT NULL
           AND NULLIF(TRIM(json_extract(t.metadata, '$.album')), '') IS NULL
           AND NOT EXISTS (SELECT 1 FROM track_edits e WHERE e.track_id=t.id AND json_type(e.fields, '$.album') IS NOT NULL)
           AND EXISTS (SELECT 1 FROM scan_sources WHERE track_id=t.id) ORDER BY t.id",
    ))?;
    let candidates = missing
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = tx.prepare_cached(
        "UPDATE track SET metadata=json_set(metadata, '$.album', ?2, '$.albumArtist', ?3)
         WHERE id=?1 RETURNING metadata",
    )?;
    for (id, path, artist) in candidates {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(folder) = Path::new(&path).parent() else {
            continue;
        };
        if let Some(Some((album, album_artist))) =
            albums.get(&(folder.to_owned(), artist.trim().to_lowercase()))
        {
            let metadata: String =
                update.query_row(params![id, album, album_artist], |row| row.get(0))?;
            queries::index(tx, id, &path, &serde_json::from_str(&metadata)?)?;
        }
    }
    Ok(())
}

fn scan(
    db: &mut Connection,
    roots: &[String],
    cancelled: &AtomicBool,
    warnings: &mut ReadWarnings,
    mut report: impl FnMut(Progress),
) -> LibraryResult<(bool, usize)> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS scan_sources (root TEXT, track_id INTEGER, PRIMARY KEY (root, track_id));
         CREATE INDEX IF NOT EXISTS temp.scan_sources_track ON scan_sources(track_id);",
    )?;
    let mut unavailable = 0;
    let mut complete = HashSet::new();
    let mut candidates = Vec::new();
    for root in roots {
        let Ok(canonical) = fs::canonicalize(root) else {
            unavailable += 1;
            continue;
        };
        if !canonical.is_dir() {
            unavailable += 1;
            continue;
        }
        let mut failed = false;
        let entries = WalkDir::new(&canonical)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0
                    || !entry.file_name().to_string_lossy().starts_with('.')
            });
        for entry in entries {
            if cancelled.load(Ordering::Acquire) {
                return Ok((true, unavailable));
            }
            match entry {
                Ok(entry) if entry.file_type().is_file() && supported(entry.path()) => {
                    if let Ok(path) = fs::canonicalize(entry.path()) {
                        if path.starts_with(&canonical) && path.to_str().is_some() {
                            let created = entry
                                .metadata()
                                .ok()
                                .and_then(|meta| meta.created().or_else(|_| meta.modified()).ok());
                            candidates.push((root, path, created));
                        }
                    } else {
                        failed = true;
                    }
                }
                Ok(_) => {}
                Err(_) => failed = true,
            }
        }
        if failed {
            unavailable += 1;
        } else {
            complete.insert(root);
        }
    }
    candidates.sort_unstable_by(|(_, path_a, created_a), (_, path_b, created_b)| {
        created_a.cmp(created_b).then_with(|| path_a.cmp(path_b))
    });
    let total = candidates.len();
    report(Progress {
        current: 0,
        total: Some(total),
    });
    let mut last_report = Instant::now();
    for (index, (root, path, _)) in candidates.into_iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Ok((true, unavailable));
        }
        if trash::has_interrupted_move(&tx, &path)? {
            continue;
        }
        let id = match index_track(&tx, &path, None, warnings) {
            Ok(id) => id,
            Err(error) if error.downcast_ref::<std::io::Error>().is_some() => {
                if complete.remove(root) {
                    unavailable += 1;
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let seen: bool = tx
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM scan_sources WHERE track_id=?1)")?
            .query_row([id], |row| row.get(0))?;
        if !seen {
            mark_catalog_addition(&tx, id)?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO scan_sources VALUES (?1, ?2)",
            params![root, id],
        )?;
        if last_report.elapsed() >= Duration::from_millis(80) || index + 1 == total {
            report(Progress {
                current: index + 1,
                total: Some(total),
            });
            last_report = Instant::now();
        }
    }
    let previous_roots = {
        let mut query = tx.prepare("SELECT DISTINCT root FROM source")?;
        let rows = query.query_map((), |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for root in previous_roots {
        if complete.contains(&root) || !roots.contains(&root) {
            tx.execute("DELETE FROM source WHERE root = ?1", [&root])?;
        }
    }
    tx.execute(
        "INSERT OR IGNORE INTO source SELECT root, track_id FROM scan_sources",
        (),
    )?;
    if roots.is_empty() || !complete.is_empty() {
        tx.execute("UPDATE catalog_state SET initialized = 1", ())?;
    }
    if cancelled.load(Ordering::Acquire) {
        return Ok((true, unavailable));
    }
    infer_scanned_albums(&tx, cancelled)?;
    if cancelled.load(Ordering::Acquire) {
        return Ok((true, unavailable));
    }
    versions::rebuild(&tx)?;
    tx.execute("UPDATE catalog_state SET revision = revision + 1", [])?;
    tx.commit()?;
    Ok((false, unavailable))
}

fn user_error(error: LibraryError) -> String {
    eprintln!("Library: {error}");
    "Не удалось обновить медиатеку".into()
}

#[tauri::command]
pub async fn library_catalog_status(state: State<'_, Library>) -> Result<CatalogStatus, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || read_status(&connect(&path)?))
        .await
        .map_err(|error| error.to_string())?
        .map_err(user_error)
}

#[tauri::command]
pub async fn library_scan(
    app: AppHandle,
    state: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    id: String,
    progress: Channel<Progress>,
    resolver: Option<crate::external_tools::ommr::Configuration>,
    resolver_url: Option<String>,
) -> Result<ScanResult, String> {
    let roots = folders
        .configured
        .lock()
        .map_err(|_| "Медиатека недоступна")?
        .clone();
    let database = state.database.clone();
    let active = Arc::clone(&state.active);
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut job = active.lock().map_err(|_| "Медиатека недоступна")?;
        if job.is_some() {
            return Err("Сканирование уже запущено".into());
        }
        *job = Some(Job {
            id,
            cancelled: Arc::clone(&cancelled),
        });
    }
    let _ = progress.send(Progress {
        current: 0,
        total: None,
    });
    let writer = state.lock_catalog_writer().await;
    let outcome = tauri::async_runtime::spawn_blocking(move || -> LibraryResult<ScanResult> {
        let _writer = writer;
        let mut db = connect(&database)?;
        let mut read_warnings = ReadWarnings::default();
        let (cancelled, unavailable_folders) =
            scan(&mut db, &roots, &cancelled, &mut read_warnings, |update| {
                let _ = progress.send(update);
            })?;
        Ok(ScanResult {
            cancelled,
            unavailable_folders,
            catalog: read_status(&db)?,
            read_warnings,
        })
    })
    .await;
    *active.lock().map_err(|_| "Медиатека недоступна")? = None;
    let outcome = outcome
        .map_err(|error| error.to_string())?
        .map_err(user_error)?;
    if !outcome.cancelled {
        state.resolver.schedule(
            app,
            state.database.clone(),
            Arc::clone(&state.catalog_writer),
            resolver.unwrap_or_else(|| crate::external_tools::ommr::Configuration {
                mode: if resolver_url
                    .as_deref()
                    .is_some_and(|url| !url.trim().is_empty())
                {
                    crate::external_tools::ommr::Mode::Remote
                } else {
                    crate::external_tools::ommr::Mode::Off
                },
                url: resolver_url,
                sources: Vec::new(),
                primary: crate::external_tools::ommr::MetadataSource::Auto,
            }),
        );
    }
    Ok(outcome)
}

#[tauri::command]
pub(crate) async fn library_enrich_download(
    state: State<'_, Library>,
    app: AppHandle,
    track_id: i64,
    resolver: crate::external_tools::ommr::Configuration,
) -> Result<(), String> {
    if track_id <= 0 {
        return Err("Недопустимый идентификатор трека".into());
    }
    state
        .resolver
        .enrich_download(
            &app,
            &state.database,
            &state.catalog_writer,
            resolver,
            track_id,
        )
        .await
        .map_err(user_error)
}

#[tauri::command]
pub fn library_cancel(state: State<'_, Library>, id: String) {
    if let Ok(job) = state.active.lock() {
        if let Some(job) = job.as_ref().filter(|job| job.id == id) {
            job.cancelled.store(true, Ordering::Release);
        }
    }
}

#[tauri::command]
pub async fn library_resolve_path(
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<Option<String>, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let path = fs::canonicalize(path).ok()?;
        roots
            .lock()
            .ok()?
            .iter()
            .any(|root| path.starts_with(root))
            .then(|| path.to_str().map(str::to_owned))
            .flatten()
    })
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path, name: &str) {
        let path = root.join(name);
        // A tiny PCM WAV file provides a real file identity and metadata.
        let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\x02\0\0\0\0\0";
        fs::write(path, wav).expect("write WAV fixture");
    }

    #[test]
    fn reopens_the_catalog_after_migrating_it() {
        let temp = tempfile::tempdir().expect("test directory");
        let database = temp.path().join("catalog.sqlite3");
        drop(Library::new(database.clone()).expect("initialize catalog"));

        let library = Library::new(database).expect("reopen migrated catalog");
        let db = connect(library.database_path()).expect("open catalog database");
        let version: u32 = db
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read catalog version");
        assert_eq!(version, CATALOG_VERSION);
    }

    #[tokio::test]
    async fn library_artwork_uses_persisted_bytes_after_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("library.sqlite3");
        let library = Library::new(database.clone()).unwrap();
        let db = connect(&database).unwrap();
        let path = Path::new("/music/track.flac");
        let url = "https://artwork-test.mzstatic.com/100x100bb.jpg";
        let tags = Tags {
            cover_url: Some(url.into()),
            ..Tags::default()
        };
        db.execute(
            "INSERT INTO track (id, path, stamp, size, metadata) VALUES (1, ?1, '0', 0, ?2)",
            rusqlite::params![
                path.to_str().unwrap(),
                serde_json::to_string(&tags).unwrap()
            ],
        )
        .unwrap();
        assert!(library.resolver_cover(path).is_none());

        let bytes = b"stored artwork";
        db.execute(
            "INSERT INTO resolver_artwork (url, bytes) VALUES (?1, ?2)",
            rusqlite::params![url, bytes.as_slice()],
        )
        .unwrap();
        drop(db);
        drop(library);

        let reopened = Library::new(database.clone()).unwrap();
        assert!(catalog::cache_artwork(&database, url).await);
        assert_eq!(
            reopened.resolver_cover(path).as_deref(),
            Some(bytes.as_slice())
        );
    }

    #[test]
    fn keeps_previous_metadata_log_entries_when_adding_sources() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("catalog.sqlite3");
        drop(Library::new(database.clone()).unwrap());
        let db = connect(&database).unwrap();
        db.execute_batch(
            "DROP TABLE release_artwork; DROP TABLE resolver_artwork;
             DROP TABLE track_edits; DROP TABLE track_trash; DROP TABLE trash_settings;
             ALTER TABLE metadata_log DROP COLUMN attempts;
             PRAGMA user_version=11;
             INSERT INTO track (id, path, stamp, size, metadata) VALUES (1, 'track', '', 0, '{}');
             INSERT INTO metadata_log VALUES (1, 'updated');",
        )
        .unwrap();
        drop(db);

        let library = Library::new(database).unwrap();
        let db = connect(library.database_path()).unwrap();
        let entry: (String, String) = db
            .query_row(
                "SELECT status, attempts FROM metadata_log WHERE track_id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(entry, ("updated".into(), "[]".into()));
    }

    #[test]
    fn scan_keeps_opus_tags_when_duration_rounds_to_zero() {
        let temp = tempfile::tempdir().expect("test directory");
        let root = temp.path().join("music");
        fs::create_dir(&root).expect("music directory");
        // One PCM sample encoded with libopus. Lofty 0.25.5 rounds it to zero milliseconds.
        fs::write(
            root.join("Short.opus"),
            include_bytes!("fixtures/one-sample.opus"),
        )
        .expect("write short Opus fixture");
        fixture(&root, "Good.wav");

        let database = temp.path().join("catalog.sqlite3");
        Library::new(database.clone()).expect("initialize catalog");
        let mut db = connect(&database).expect("open catalog");
        let roots = vec![root.to_string_lossy().into_owned()];
        let mut warnings = ReadWarnings::default();
        scan(
            &mut db,
            &roots,
            &AtomicBool::new(false),
            &mut warnings,
            |_| {},
        )
        .expect("scan");

        let catalog = read_catalog(&db).expect("read catalog");
        let names: Vec<_> = catalog
            .tracks
            .iter()
            .map(|track| track.tags.name.as_str())
            .collect();
        assert_eq!(names, ["Good", "Short track"]);
        assert_eq!(warnings.count, 1);
    }

    #[test]
    fn duration_probe_recovers_from_a_wav_with_zero_sample_rate() {
        let temp = tempfile::tempdir().expect("test directory");
        fixture(temp.path(), "Broken.wav");
        let broken = temp.path().join("Broken.wav");
        let mut bytes = fs::read(&broken).expect("read WAV fixture");
        bytes[24..28].copy_from_slice(&0_u32.to_le_bytes());
        fs::write(&broken, bytes).expect("write WAV with zero sample rate");

        let mut warnings = ReadWarnings::default();
        let tags = read_tags(&broken, &mut warnings);
        assert!(tags.duration.is_none());
        assert_eq!(warnings.count, 1);
    }

    #[test]
    fn overlapping_and_offline_folders_preserve_track_identity() {
        let temp = tempfile::tempdir().expect("test directory");
        let root = temp.path().join("music");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("music directories");
        fixture(&nested, "Song.wav");
        let database = temp.path().join("catalog.sqlite3");
        Library::new(database.clone()).expect("initialize catalog");
        let mut db = connect(&database).expect("open catalog");
        let roots = vec![
            root.to_string_lossy().into_owned(),
            nested.to_string_lossy().into_owned(),
        ];
        let running = AtomicBool::new(false);
        scan(
            &mut db,
            &roots,
            &running,
            &mut ReadWarnings::default(),
            |_| {},
        )
        .expect("scan");
        let first = read_catalog(&db).expect("read catalog");
        assert_eq!(first.tracks.len(), 1);

        fs::rename(&root, temp.path().join("offline")).expect("simulate disconnected folder");
        assert_eq!(
            scan(
                &mut db,
                &roots,
                &running,
                &mut ReadWarnings::default(),
                |_| {}
            )
            .expect("rescan"),
            (false, 2)
        );
        let second = read_catalog(&db).expect("read retained catalog");
        assert_eq!(second.tracks[0].id, first.tracks[0].id);
    }

    #[test]
    fn marks_readded_tracks_as_new_without_reordering_tracks_during_rescan() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("catalog.sqlite3");
        Library::new(database.clone()).unwrap();
        let mut db = connect(&database).unwrap();
        let tx = db.transaction().unwrap();
        tx.execute_batch(
            "INSERT INTO track (id, path, stamp, size, metadata, added_at)
             VALUES (1, 'first', '', 0, '{}', 100), (2, 'second', '', 0, '{}', 200);
             INSERT INTO source VALUES ('test', 1), ('test', 2);",
        )
        .unwrap();
        let added = |id| {
            tx.query_row("SELECT added_at FROM track WHERE id=?1", [id], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        };
        mark_catalog_addition(&tx, 1).unwrap();
        assert_eq!(added(1), 100);
        tx.execute("DELETE FROM source WHERE track_id=1", [])
            .unwrap();
        mark_catalog_addition(&tx, 1).unwrap();
        assert!(added(1) > added(2));
    }

    #[test]
    fn infers_an_album_from_a_matching_folder_and_credited_artist() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("catalog.sqlite3");
        Library::new(database.clone()).unwrap();
        let mut db = connect(&database).unwrap();
        let tx = db.transaction().unwrap();
        tx.execute_batch("CREATE TEMP TABLE scan_sources (root TEXT, track_id INTEGER);")
            .unwrap();
        for (id, artist, album) in [
            (1, "Artist", Some("Album")),
            (2, "Artist", None),
            (3, "Other", None),
        ] {
            let tags = Tags {
                name: format!("Song {id}"),
                artists: vec![artist.into()],
                album: album.map(str::to_owned),
                ..Tags::default()
            };
            let path = format!("/music/Album/track{id}.flac");
            tx.execute(
                "INSERT INTO track (id, path, stamp, size, metadata) VALUES (?1, ?2, '', 0, ?3)",
                params![id, path, serde_json::to_string(&tags).unwrap()],
            )
            .unwrap();
            tx.execute("INSERT INTO scan_sources VALUES ('/music', ?1)", [id])
                .unwrap();
            queries::index(&tx, id, &path, &tags).unwrap();
        }
        infer_scanned_albums(&tx, &AtomicBool::new(false)).unwrap();
        let read_album = |id| {
            tx.query_row(
                "SELECT json_extract(metadata, '$.album') FROM track WHERE id=?1",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
        };
        assert_eq!(read_album(2).as_deref(), Some("Album"));
        assert_eq!(read_album(3), None);
    }

    #[test]
    fn rescan_preserves_the_number_of_completed_plays() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("music");
        fs::create_dir(&root).unwrap();
        fixture(&root, "Song.wav");
        let database = temp.path().join("catalog.sqlite3");
        let library = Library::new(database.clone()).unwrap();
        let mut db = connect(&database).unwrap();
        let roots = [root.to_string_lossy().into_owned()];
        let cancelled = AtomicBool::new(false);
        let mut warnings = ReadWarnings::default();
        scan(&mut db, &roots, &cancelled, &mut warnings, |_| {}).unwrap();
        let track = read_catalog(&db).unwrap().tracks.remove(0);
        assert_eq!(track.play_count, 0);
        library.record_play(&track.path).unwrap();
        library.record_play(&track.path).unwrap();
        db.execute("UPDATE track SET stamp='modified'", []).unwrap();
        scan(&mut db, &roots, &cancelled, &mut warnings, |_| {}).unwrap();
        assert_eq!(read_catalog(&db).unwrap().tracks[0].play_count, 2);
    }

    #[test]
    fn cancellation_rolls_back_catalog_changes() {
        let temp = tempfile::tempdir().expect("test directory");
        let root = temp.path().join("music");
        fs::create_dir(&root).expect("music directory");
        fixture(&root, "Original.wav");
        let database = temp.path().join("catalog.sqlite3");
        Library::new(database.clone()).expect("initialize catalog");
        let mut db = connect(&database).expect("open catalog");
        let roots = vec![root.to_string_lossy().into_owned()];
        let cancelled = AtomicBool::new(false);
        scan(
            &mut db,
            &roots,
            &cancelled,
            &mut ReadWarnings::default(),
            |_| {},
        )
        .expect("initial scan");

        fs::remove_file(root.join("Original.wav")).expect("remove fixture");
        fixture(&root, "New.wav");
        let outcome = scan(
            &mut db,
            &roots,
            &cancelled,
            &mut ReadWarnings::default(),
            |progress| {
                if progress.current == 1 {
                    cancelled.store(true, Ordering::Release);
                }
            },
        )
        .expect("cancelled scan");
        assert!(outcome.0);
        let catalog = read_catalog(&db).expect("read retained catalog");
        assert_eq!(catalog.tracks.len(), 1);
        assert_eq!(catalog.tracks[0].tags.name, "Original");
    }
}
