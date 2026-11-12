pub mod playlists;
pub mod queries;
pub(crate) mod youtube;

use std::{
    collections::HashSet,
    fs,
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
use tauri::{ipc::Channel, State};
use walkdir::WalkDir;

use crate::{audio, MusicFolders};

type LibraryError = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type LibraryResult<T> = Result<T, LibraryError>;

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Tags {
    name: String,
    artists: Vec<String>,
    lyrics: Vec<String>,
    album: Option<String>,
    album_artist: Option<String>,
    year: Option<u16>,
    duration: Option<f64>,
    replay_gain_db: Option<f32>,
    disc_number: Option<u32>,
    track_number: Option<u32>,
    genre: Option<String>,
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
    #[serde(flatten)]
    tags: Tags,
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
}

impl Library {
    pub fn new(database: PathBuf) -> LibraryResult<Self> {
        if let Some(parent) = database.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut connection = connect(&database)?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 4 {
            return Err("Unsupported catalog version".into());
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
            youtube::migrate(&mut connection)?;
        }
        Ok(Self {
            database,
            active: Arc::default(),
            catalog_writer: Arc::default(),
        })
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
        .unwrap_or_else(|| read_tags(path));
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
        download: youtube::Download<'_>,
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
        let id = index_track(&tx, path, Some(download.tags))?;
        youtube::record(&tx, id, download)?;
        for root in sources {
            tx.execute(
                "INSERT OR IGNORE INTO source VALUES (?1, ?2)",
                params![root, id],
            )?;
        }
        let (path, tags) = tx.query_row(
            "SELECT path, metadata FROM track WHERE id = ?1",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        let track = Track {
            id,
            path,
            tags: serde_json::from_str(&tags)?,
        };
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

fn read_status(db: &Connection) -> LibraryResult<CatalogStatus> {
    Ok(db.query_row(
        "SELECT initialized, (SELECT COUNT(DISTINCT track_id) FROM source) FROM catalog_state",
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
        "SELECT id, path, metadata FROM track
         WHERE EXISTS (SELECT 1 FROM source WHERE track_id = track.id)
         ORDER BY path",
    )?;
    let mut rows = query.query(())?;
    let mut tracks = Vec::new();
    while let Some(row) = rows.next()? {
        let metadata: String = row.get(2)?;
        tracks.push(Track {
            id: row.get(0)?,
            path: row.get(1)?,
            tags: serde_json::from_str(&metadata)?,
        });
    }
    Ok(Catalog { tracks })
}

pub(crate) fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp3" | "m4a" | "ogg" | "opus" | "webm" | "wav" | "flac"
            )
        })
}

fn read_tags(path: &Path) -> Tags {
    let mut tags = Tags {
        name: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Tags::default()
    };
    let options = ParseOptions::new().read_cover_art(false);
    if let Ok(file) = Probe::open(path).and_then(|probe| probe.options(options).read()) {
        let duration = file.properties().duration().as_secs_f64();
        tags.duration = (duration > 0.0).then_some(duration);
        if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
            if let Some(title) = tag.title().filter(|title| !title.trim().is_empty()) {
                tags.name = title.into_owned();
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
            tags.year = tag.date().map(|date| date.year);
            tags.track_number = tag.track();
            tags.disc_number = tag.disk();
            tags.genre = tag.genre().map(|genre| genre.into_owned());
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
        tags.duration = audio::source::open(path).ok().and_then(|source| {
            source
                .total_duration()
                .map(|duration| duration.as_secs_f64())
        });
    }
    tags
}

fn index_track(
    tx: &Transaction<'_>,
    path: &Path,
    downloaded_tags: Option<&youtube::DownloadTags>,
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
                "SELECT id FROM track WHERE path = ?1 AND stamp = ?2 AND size = ?3",
                params![key, stamp, size],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
    }
    let mut tags = read_tags(path);
    if let Some(downloaded) = downloaded_tags {
        downloaded.apply(&mut tags);
    } else if let Some(downloaded) = youtube::stored_tags(tx, key)? {
        downloaded.apply(&mut tags);
    }
    let id = tx.query_row(
        "INSERT INTO track (path, stamp, size, metadata) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET stamp=excluded.stamp, size=excluded.size, metadata=excluded.metadata
         RETURNING id",
        params![key, stamp, size, serde_json::to_string(&tags)?],
        |row| row.get(0),
    )?;
    queries::index(tx, id, key, &tags)?;
    Ok(id)
}

fn scan(
    db: &mut Connection,
    roots: &[String],
    cancelled: &AtomicBool,
    mut report: impl FnMut(Progress),
) -> LibraryResult<(bool, usize)> {
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS scan_sources (root TEXT, track_id INTEGER, PRIMARY KEY (root, track_id));")?;
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
                entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
            });
        for entry in entries {
            if cancelled.load(Ordering::Acquire) {
                return Ok((true, unavailable));
            }
            match entry {
                Ok(entry) if entry.file_type().is_file() && supported(entry.path()) => {
                    if let Ok(path) = fs::canonicalize(entry.path()) {
                        if path.starts_with(&canonical) && path.to_str().is_some() {
                            candidates.push((root, path));
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
    let total = candidates.len();
    report(Progress {
        current: 0,
        total: Some(total),
    });
    let mut last_report = Instant::now();
    for (index, (root, path)) in candidates.into_iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Ok((true, unavailable));
        }
        let id = match index_track(&tx, &path, None) {
            Ok(id) => id,
            Err(error) if error.downcast_ref::<std::io::Error>().is_some() => {
                if complete.remove(root) {
                    unavailable += 1;
                }
                continue;
            }
            Err(error) => return Err(error),
        };
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
    state: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    id: String,
    progress: Channel<Progress>,
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
        let (cancelled, unavailable_folders) = scan(&mut db, &roots, &cancelled, |update| {
            let _ = progress.send(update);
        })?;
        Ok(ScanResult {
            cancelled,
            unavailable_folders,
            catalog: read_status(&db)?,
        })
    })
    .await;
    *active.lock().map_err(|_| "Медиатека недоступна")? = None;
    outcome
        .map_err(|error| error.to_string())?
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
        scan(&mut db, &roots, &running, |_| {}).expect("scan");
        let first = read_catalog(&db).expect("read catalog");
        assert_eq!(first.tracks.len(), 1);

        fs::rename(&root, temp.path().join("offline")).expect("simulate disconnected folder");
        assert_eq!(
            scan(&mut db, &roots, &running, |_| {}).expect("rescan"),
            (false, 2)
        );
        let second = read_catalog(&db).expect("read retained catalog");
        assert_eq!(second.tracks[0].id, first.tracks[0].id);
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
        scan(&mut db, &roots, &cancelled, |_| {}).expect("initial scan");

        fs::remove_file(root.join("Original.wav")).expect("remove fixture");
        fixture(&root, "New.wav");
        let outcome = scan(&mut db, &roots, &cancelled, |progress| {
            if progress.current == 1 {
                cancelled.store(true, Ordering::Release);
            }
        })
        .expect("cancelled scan");
        assert!(outcome.0);
        let catalog = read_catalog(&db).expect("read retained catalog");
        assert_eq!(catalog.tracks.len(), 1);
        assert_eq!(catalog.tracks[0].tags.name, "Original");
    }
}
