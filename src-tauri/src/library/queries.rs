use std::collections::HashSet;

use icu_collator::{
    options::{CollatorOptions, Strength},
    preferences::CollationNumericOrdering,
    Collator, CollatorPreferences,
};
use icu_locale_core::locale;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use super::{catalog, connect, user_error, Library, LibraryResult, Tags, Track};
use crate::audio::{Action, AudioEngine, Snapshot};
use crate::{external_tools::valid_video_id, MusicFolders};

pub(super) fn migrate(db: &mut Connection) -> LibraryResult<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "ALTER TABLE catalog_state ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
        CREATE TABLE track_index (
            id INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
            name_key BLOB NOT NULL,
            artist_key BLOB NOT NULL,
            path_key BLOB NOT NULL,
            duration REAL NOT NULL,
            gain REAL,
            searchable TEXT NOT NULL,
            album_id TEXT,
            album_name TEXT,
            album_artist TEXT,
            disc INTEGER NOT NULL,
            track_number INTEGER NOT NULL
        );
        CREATE INDEX track_name_order ON track_index(name_key, path_key, id);
        CREATE INDEX track_artist_order ON track_index(artist_key, name_key, path_key, id);
        CREATE INDEX track_duration_order ON track_index(duration, name_key, path_key, id);
        CREATE INDEX track_album ON track_index(album_id, disc, track_number, name_key);
        CREATE TABLE track_artist (
            name_key TEXT NOT NULL,
            track_id INTEGER NOT NULL REFERENCES track(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            PRIMARY KEY (name_key, track_id)
        );
        CREATE INDEX track_artist_track ON track_artist(track_id);
        CREATE VIRTUAL TABLE track_search USING fts5(
            searchable, content='track_index', content_rowid='id', tokenize='trigram case_sensitive 1'
        );
        CREATE TRIGGER track_index_insert AFTER INSERT ON track_index BEGIN
            INSERT INTO track_search(rowid, searchable) VALUES(new.id, new.searchable);
        END;
        CREATE TRIGGER track_index_update AFTER UPDATE OF searchable ON track_index
        WHEN new.searchable IS NOT old.searchable BEGIN
            INSERT INTO track_search(track_search, rowid, searchable) VALUES('delete', old.id, old.searchable);
            INSERT INTO track_search(rowid, searchable) VALUES(new.id, new.searchable);
        END;
        CREATE TRIGGER track_index_delete AFTER DELETE ON track_index BEGIN
            INSERT INTO track_search(track_search, rowid, searchable) VALUES('delete', old.id, old.searchable);
        END;",
    )?;
    {
        let mut statement = tx.prepare("SELECT id, path, metadata FROM track")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let path: String = row.get(1)?;
            let metadata: String = row.get(2)?;
            index(&tx, row.get(0)?, &path, &serde_json::from_str(&metadata)?)?;
        }
    }
    tx.pragma_update(None, "user_version", 3)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn migrate_added_date(db: &mut Connection) -> LibraryResult<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "ALTER TABLE track ADD COLUMN added_at INTEGER;
         CREATE INDEX track_added_order ON track(-COALESCE(added_at, 0), -id);
         PRAGMA user_version = 6;",
    )?;
    tx.commit()?;
    Ok(())
}

thread_local! {
    static COLLATOR: Result<icu_collator::CollatorBorrowed<'static>, String> = {
        let mut preferences: CollatorPreferences = locale!("ru").into();
        preferences.numeric_ordering = Some(CollationNumericOrdering::True);
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Primary);
        Collator::try_new(preferences, options).map_err(|error| error.to_string())
    };
}

pub(super) fn migrate_singles(db: &mut Connection) -> LibraryResult<()> {
    let tx = db.transaction()?;
    let mut cursor = 0;
    loop {
        let rows = {
            let mut statement = tx.prepare_cached(
                "SELECT t.id, t.path, t.metadata, y.metadata, y.video_id FROM track t
                 LEFT JOIN external_download y ON y.track_id=t.id
                 WHERE t.id>?1 ORDER BY t.id LIMIT 256",
            )?;
            let rows = statement.query_map([cursor], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            break;
        }
        for (id, path, metadata, download, video_id) in rows {
            cursor = id;
            let mut tags: Tags = serde_json::from_str(&metadata)?;
            tags.external_video_id = video_id.or_else(|| {
                super::external::legacy_id(std::path::Path::new(&path)).map(str::to_owned)
            });
            if let Some(download) = download {
                let mut download: super::external::DownloadTags = serde_json::from_str(&download)?;
                let old_title = download.title.clone();
                let old_artists = download.artists.clone();
                download.normalize();
                if tags.name == old_title {
                    tags.name.clone_from(&download.title);
                }
                if tags.artists == old_artists {
                    tags.artists.clone_from(&download.artists);
                }
                if download.title != old_title || download.artists != old_artists {
                    tx.execute(
                        "UPDATE external_download SET metadata=?2 WHERE track_id=?1",
                        params![id, serde_json::to_string(&download)?],
                    )?;
                }
            }
            if tags.external_video_id.is_some() {
                tx.execute(
                    "UPDATE track SET metadata=?2 WHERE id=?1",
                    params![id, serde_json::to_string(&tags)?],
                )?;
            }
            index(&tx, id, &path, &tags)?;
        }
    }
    tx.execute_batch("UPDATE catalog_state SET revision=revision+1; PRAGMA user_version=7;")?;
    tx.commit()?;
    Ok(())
}

pub(super) fn index(db: &Transaction<'_>, id: i64, path: &str, tags: &Tags) -> LibraryResult<()> {
    let artists = tags.artists.join(", ");
    let album = tags.album_name().unwrap_or(&tags.name);
    let album_artist = tags
        .album_artist
        .as_deref()
        .map(str::trim)
        .filter(|artist| !artist.is_empty())
        .unwrap_or(&artists);
    let album_id = tags.album_id(id)?;
    let searchable = format!(
        "{} {} {}",
        tags.name,
        tags.album.as_deref().unwrap_or(""),
        tags.artists.join(" ")
    )
    .to_lowercase();
    let (name_key, artist_key, path_key) = COLLATOR.with(|collator| -> LibraryResult<_> {
        let collator = collator
            .as_ref()
            .map_err(|error| std::io::Error::other(error.clone()))?;
        let mut name = Vec::new();
        let mut artist = Vec::new();
        let mut file = Vec::new();
        collator.write_sort_key_to(&tags.name, &mut name)?;
        collator.write_sort_key_to(&artists, &mut artist)?;
        collator.write_sort_key_to(path, &mut file)?;
        Ok((name, artist, file))
    })?;
    db.prepare_cached(
        "INSERT INTO track_index VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(id) DO UPDATE SET name_key=excluded.name_key, artist_key=excluded.artist_key,
         path_key=excluded.path_key, duration=excluded.duration, gain=excluded.gain,
         searchable=excluded.searchable, album_id=excluded.album_id, album_name=excluded.album_name,
         album_artist=excluded.album_artist, disc=excluded.disc, track_number=excluded.track_number",
    )?.execute(
        params![id, name_key, artist_key, path_key,
            tags.duration.filter(|value| value.is_finite() && *value > 0.0).unwrap_or(f64::INFINITY),
            tags.replay_gain_db, searchable, album_id, album, album_artist,
            tags.disc_number.unwrap_or(1), tags.track_number.map(i64::from).unwrap_or(i64::MAX)],
    )?;
    db.prepare_cached("DELETE FROM track_artist WHERE track_id = ?1")?
        .execute([id])?;
    let mut artist = db.prepare_cached("INSERT OR IGNORE INTO track_artist VALUES (?1, ?2, ?3)")?;
    for name in &tags.artists {
        let key = name.trim().to_lowercase();
        if !key.is_empty() {
            artist.execute(params![key, id, name])?;
        }
    }
    Ok(())
}

const ACTIVE: &str = "EXISTS (SELECT 1 FROM source WHERE track_id = t.id)";
const PRIMARY: &str = "NOT EXISTS (SELECT 1 FROM track_variant WHERE variant_id=t.id)";
const PAGE_SIZE: usize = 48;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sort {
    Name,
    Artist,
    Duration,
    Added,
    Path,
}

impl Sort {
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Name => &["i.name_key", "i.path_key", "i.id"],
            Self::Artist => &["i.artist_key", "i.name_key", "i.path_key", "i.id"],
            Self::Duration => &["i.duration", "i.name_key", "i.path_key", "i.id"],
            // Negative keys keep cursor comparisons ascending while showing recent additions first.
            Self::Added => &["-COALESCE(t.added_at, 0)", "-t.id"],
            Self::Path => &["t.path", "i.id"],
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackQuery {
    sort: Sort,
    text: String,
    exclude_playlist: Option<i64>,
}

impl TrackQuery {
    fn terms(&self) -> LibraryResult<(String, &'static str, String)> {
        if self.text.len() > 2048 || self.text.contains('\0') {
            return Err("Invalid search text".into());
        }
        let text = self.text.trim().to_lowercase();
        let words = search_words(&text);
        if words.len() > 64 {
            return Err("Invalid search text".into());
        }
        let (long, mut short): (Vec<_>, Vec<_>) = words
            .into_iter()
            .partition(|word| word.chars().count() >= 3);
        if long.is_empty() && short.is_empty() && !text.is_empty() {
            short.push(&text);
        }
        let term = long
            .iter()
            .map(|word| format!("\"{word}\""))
            .collect::<Vec<_>>()
            .join(" AND ");
        let predicate = if term.is_empty() {
            "?1 = ''"
        } else {
            "i.id IN (SELECT rowid FROM track_search WHERE track_search MATCH ?1)"
        };
        Ok((term, predicate, serde_json::to_string(&short)?))
    }

    fn select(&self, fields: &str, after: bool) -> LibraryResult<(String, String, String, String)> {
        let (term, predicate, short) = self.terms()?;
        let phrase = self
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let mut keys = self.sort.keys().join(", ");
        if phrase.contains(' ') {
            keys.insert_str(0, "CASE WHEN instr(i.searchable, ?6)>0 THEN 0 ELSE 1 END, ");
        }
        let cursor = if after {
            format!("AND ({keys}) > (SELECT {keys} FROM track_index i JOIN track t ON t.id=i.id WHERE i.id=?2)")
        } else {
            String::new()
        };
        Ok((
            format!(
                "SELECT {fields} FROM track_index i JOIN track t ON t.id=i.id
             WHERE {ACTIVE} AND {PRIMARY} AND {predicate} {cursor}
             AND ?6 IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM json_each(?5) w WHERE instr(i.searchable, w.value)=0)
             AND (?4 IS NULL OR NOT EXISTS (
                 SELECT 1 FROM playlist_track WHERE playlist_id=?4 AND track_id=i.id
             ))
             ORDER BY {keys} LIMIT ?3"
            ),
            term,
            short,
            phrase,
        ))
    }
}

pub(crate) fn search_words(text: &str) -> Vec<&str> {
    let mut seen = HashSet::new();
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty() && seen.insert(*word))
        .collect()
}

#[derive(Deserialize, Serialize)]
pub struct Cursor {
    id: i64,
    revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    tracks: Vec<Track>,
    next_cursor: Option<Cursor>,
    revision: i64,
}

fn track_row(row: &rusqlite::Row<'_>) -> LibraryResult<Track> {
    Track::from_metadata(
        row.get(0)?,
        row.get(1)?,
        &row.get::<_, String>(2)?,
        row.get(3)?,
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEntry {
    id: i64,
    played_at: i64,
    track: Track,
}

#[tauri::command]
pub async fn library_playback_history(
    state: State<'_, Library>,
    before: Option<i64>,
) -> Result<Vec<PlaybackEntry>, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<Vec<PlaybackEntry>> {
        let db = connect(&database)?;
        let mut statement = db.prepare(&format!(
            "SELECT t.id, t.path, {}, {}, h.id, h.played_at
             FROM playback_history h
             LEFT JOIN track_variant v ON v.variant_id=h.track_id
             JOIN track t ON t.id=COALESCE(v.audio_id,h.track_id)
             WHERE h.id<COALESCE(?1,9223372036854775807) AND {ACTIVE}
             ORDER BY h.id DESC LIMIT 80",
            super::versions::METADATA,
            super::versions::PLAY_COUNT,
        ))?;
        let mut rows = statement.query([before])?;
        let mut history = Vec::new();
        while let Some(row) = rows.next()? {
            history.push(PlaybackEntry {
                id: row.get(4)?,
                played_at: row.get(5)?,
                track: track_row(row)?,
            });
        }
        Ok(history)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataLogPage {
    entries: Vec<MetadataLogEntry>,
    scanned: i64,
    total: i64,
    updated: i64,
    issues: i64,
}

#[derive(Serialize)]
pub struct MetadataLogEntry {
    track: Track,
    status: String,
    attempts: Vec<super::ommr::Attempt>,
}

#[tauri::command]
pub async fn library_metadata_log(
    state: State<'_, Library>,
    folder: String,
    before: Option<i64>,
) -> Result<MetadataLogPage, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<MetadataLogPage> {
        let db = connect(&database)?;
        let (total, scanned, updated, issues) = db.query_row(
            "SELECT COUNT(*), COUNT(CASE WHEN m.status<>'searching' THEN 1 END),
                    COUNT(CASE WHEN m.status='updated' THEN 1 END),
                    COUNT(CASE WHEN m.status IN ('error', 'unavailable') THEN 1 END) FROM source s
             LEFT JOIN metadata_log m ON m.track_id=s.track_id WHERE s.root=?1",
            [&folder],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let mut statement = db.prepare(&format!(
            "SELECT t.id, t.path, {}, {}, m.status, m.attempts FROM source s
             JOIN metadata_log m ON m.track_id=s.track_id JOIN track t ON t.id=m.track_id
             WHERE s.root=?1 AND t.id<COALESCE(?2,9223372036854775807) ORDER BY t.id DESC LIMIT 50",
            super::versions::METADATA,
            super::versions::PLAY_COUNT,
        ))?;
        let mut rows = statement.query(params![folder, before])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(MetadataLogEntry {
                track: track_row(row)?,
                status: row.get(4)?,
                attempts: serde_json::from_str(&row.get::<_, String>(5)?)?,
            });
        }
        Ok(MetadataLogPage {
            entries,
            scanned,
            total,
            updated,
            issues,
        })
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)
}

fn read_page(
    db: &mut Connection,
    query: &TrackQuery,
    cursor: Option<Cursor>,
) -> LibraryResult<Page> {
    let tx = db.transaction()?;
    let revision = tx.query_row("SELECT revision FROM catalog_state", [], |row| row.get(0))?;
    let after = cursor
        .filter(|cursor| cursor.revision == revision)
        .map(|cursor| cursor.id);
    let (sql, term, short, phrase) = query.select(
        &format!(
            "t.id, t.path, {}, {}",
            super::versions::METADATA,
            super::versions::PLAY_COUNT
        ),
        after.is_some(),
    )?;
    let mut statement = tx.prepare(&sql)?;
    let mut rows = statement.query(params![
        term,
        after,
        PAGE_SIZE as i64 + 1,
        query.exclude_playlist,
        short,
        phrase
    ])?;
    let mut tracks = Vec::new();
    while let Some(row) = rows.next()? {
        tracks.push(track_row(row)?);
    }
    let next_cursor = if tracks.len() > PAGE_SIZE {
        tracks.pop();
        tracks.last().map(|track| Cursor {
            id: track.id,
            revision,
        })
    } else {
        None
    };
    Ok(Page {
        tracks,
        next_cursor,
        revision,
    })
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum QueueTarget {
    Id { id: i64 },
    Path { path: String },
    First,
    Last,
}

struct QueueTrack {
    id: i64,
    path: String,
    gain: Option<f32>,
}

impl QueueTrack {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            path: row.get(1)?,
            gain: row.get(2)?,
        })
    }

    fn into_audio(self) -> crate::audio::Track {
        crate::audio::Track::local(self.path, self.gain)
    }
}

fn query_queue(db: &Connection, query: &TrackQuery) -> LibraryResult<Vec<QueueTrack>> {
    let (sql, term, short, phrase) = query.select("t.id, t.path, i.gain", false)?;
    let mut statement = db.prepare(&sql)?;
    let rows = statement.query_map(
        params![term, None::<i64>, -1, query.exclude_playlist, short, phrase],
        QueueTrack::from_row,
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn read_queue(
    db: &Connection,
    query: &TrackQuery,
    target: QueueTarget,
) -> LibraryResult<Option<(Vec<crate::audio::Track>, usize)>> {
    let target = match target {
        QueueTarget::Id { id } => QueueTarget::Id { id: super::versions::audio_id(db, id)? },
        QueueTarget::Path { path } => QueueTarget::Path { path: db.query_row(
            "SELECT a.path FROM track v JOIN track_variant r ON r.variant_id=v.id JOIN track a ON a.id=r.audio_id WHERE v.path=?1",
            [&path], |row| row.get(0)).optional()?.unwrap_or(path) },
        other => other,
    };
    let entries = query_queue(db, query)?;
    let selected = match target {
        QueueTarget::Id { id } => entries.iter().position(|entry| entry.id == id),
        QueueTarget::Path { path } => entries.iter().position(|entry| entry.path == path),
        QueueTarget::First => (!entries.is_empty()).then_some(0),
        QueueTarget::Last => entries.len().checked_sub(1),
    };
    Ok(selected.map(|index| {
        (
            entries.into_iter().map(QueueTrack::into_audio).collect(),
            index,
        )
    }))
}

fn read_search_queue(
    db: &Connection,
    query: &TrackQuery,
    track_id: i64,
    linked_ids: impl IntoIterator<Item = i64>,
) -> LibraryResult<Option<Vec<QueueTrack>>> {
    let selected = super::versions::audio_id(db, track_id)?;
    let mut entries = query_queue(db, query)?;
    let mut seen: HashSet<_> = entries.iter().map(|entry| entry.id).collect();
    let mut lookup = db.prepare(&format!(
        "SELECT t.id, t.path, i.gain FROM track_index i JOIN track t ON t.id=i.id
         WHERE t.id=?1 AND {ACTIVE} AND {PRIMARY}"
    ))?;
    for id in linked_ids {
        if seen.insert(id) {
            entries.extend(lookup.query_row([id], QueueTrack::from_row).optional()?);
        }
    }
    if !entries.iter().any(|entry| entry.id == selected) {
        let Some(track) = lookup
            .query_row([selected], QueueTrack::from_row)
            .optional()?
        else {
            return Ok(None);
        };
        entries.insert(0, track);
    }
    if entries.len() == 1 {
        // A search with only one playable result can continue through the library order.
        entries = query_queue(
            db,
            &TrackQuery {
                sort: Sort::Path,
                text: String::new(),
                exclude_playlist: None,
            },
        )?;
    }
    let Some(index) = entries.iter().position(|entry| entry.id == selected) else {
        return Ok(None);
    };
    entries.rotate_left(index);
    Ok(Some(entries))
}

#[tauri::command]
pub async fn library_tracks_page(
    state: State<'_, Library>,
    query: TrackQuery,
    cursor: Option<Cursor>,
) -> Result<Page, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || read_page(&mut connect(&path)?, &query, cursor))
        .await
        .map_err(|error| error.to_string())?
        .map_err(user_error)
}

#[tauri::command]
pub async fn library_play_tracks(
    app: AppHandle,
    state: State<'_, Library>,
    audio: State<'_, AudioEngine>,
    folders: State<'_, MusicFolders>,
    query: TrackQuery,
    target: QueueTarget,
    position: f64,
    playing: bool,
    search_video_ids: Option<Vec<String>>,
) -> Result<Option<Snapshot>, String> {
    let path = state.database.clone();
    let search = if let Some(ids) = search_video_ids {
        let QueueTarget::Id { id } = target else {
            return Err("Для очереди поиска нужен выбранный трек".into());
        };
        if ids.len() > 24 || !ids.iter().all(|id| valid_video_id(id)) {
            return Err("Некорректные идентификаторы видео".into());
        }
        let roots = folders
            .canonical
            .lock()
            .map_err(|error| error.to_string())?
            .clone();
        Some((id, ids, roots))
    } else {
        None
    };
    let restart_shuffle = search.is_some();
    let request = audio.reserve_queue();
    let queue = tauri::async_runtime::spawn_blocking(move || -> LibraryResult<_> {
        let mut db = connect(&path)?;
        let Some((id, ids, roots)) = search else {
            return read_queue(&db, &query, target);
        };
        let linked = if ids.is_empty() {
            Default::default()
        } else {
            app.state::<Library>()
                .active_external_tracks(&ids, &roots)?
        };
        let linked_ids = ids
            .iter()
            .filter_map(|video| linked.get(video).map(|track| track.id));
        let tx = db.transaction()?;
        Ok(read_search_queue(&tx, &query, id, linked_ids)?
            .map(|entries| (entries.into_iter().map(QueueTrack::into_audio).collect(), 0)))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)?;
    let Some((tracks, index)) = queue else {
        return Ok(None);
    };
    audio
        .control_reserved_queue(
            request,
            Action::Queue {
                tracks,
                index,
                position,
                playing,
                restart_shuffle,
                playlist_mix: None,
            },
        )
        .await
        .map(Some)
}

#[tauri::command]
pub async fn library_tracks_by_paths(
    state: State<'_, Library>,
    paths: Vec<String>,
) -> Result<Vec<Track>, String> {
    if paths.len() > 100 {
        return Err("Слишком много треков в запросе".into());
    }
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<Vec<Track>> {
        let db = connect(&database)?;
        let mut statement = db.prepare(&format!(
            "SELECT t.id, t.path, {}, {} FROM track t WHERE t.path=?1",
            super::versions::METADATA,
            super::versions::PLAY_COUNT
        ))?;
        let mut tracks = Vec::new();
        for path in paths {
            let mut rows = statement.query([path])?;
            if let Some(row) = rows.next()? {
                tracks.push(track_row(row)?);
            }
        }
        Ok(tracks)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollectionKind {
    Album,
    Artist,
    Playlist,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    id: String,
    name: String,
    subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artists: Option<Vec<String>>,
    short_release: bool,
    cover_path: Option<String>,
    duration: f64,
    track_count: i64,
    year: Option<u16>,
    added_at: Option<i64>,
    first_track_id: Option<i64>,
}

impl Library {
    pub(crate) fn album_cover_paths(&self, path: &str) -> LibraryResult<Vec<String>> {
        let db = connect(&self.database)?;
        let mut statement = db.prepare(&format!(
            "SELECT t.path FROM track_index i JOIN track t ON t.id=i.id
             WHERE i.album_id=(
                SELECT member.album_id FROM track original
                LEFT JOIN track_variant v ON v.variant_id=original.id
                JOIN track_index member ON member.id=COALESCE(v.audio_id, original.id)
                WHERE original.path=?1
             ) AND i.album_id NOT LIKE 'single:%' AND {ACTIVE} AND {PRIMARY}
             ORDER BY i.disc, i.track_number, i.name_key, t.path"
        ))?;
        let rows = statement.query_map([path], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn collection_summaries(
    db: &Connection,
    kind: CollectionKind,
    text: &str,
) -> LibraryResult<Vec<CollectionSummary>> {
    let sql = match kind {
        CollectionKind::Album => format!(
            "SELECT i.album_id, i.album_name, i.album_artist, t.path,
                    SUM(CASE WHEN i.duration < 1e308 THEN i.duration ELSE 0 END) OVER album,
                    COUNT(*) OVER album, MAX(json_extract(t.metadata, '$.year')) OVER album,
                    NULLIF(MIN(COALESCE(t.added_at, 0)) OVER album, 0), MIN(t.id) OVER album,
                    json_extract(t.metadata, '$.artists'),
                    MAX(CASE WHEN json_extract(t.metadata, '$.isSingle') = 1
                                   OR json_extract(t.metadata, '$.isEp') = 1 THEN 1 ELSE 0 END) OVER album
             FROM track_index i JOIN track t ON t.id=i.id
             WHERE i.album_id IS NOT NULL AND {ACTIVE} AND {PRIMARY}
             WINDOW album AS (PARTITION BY i.album_id)
             ORDER BY i.album_id, i.disc, i.track_number, i.name_key, t.path"
        ),
        CollectionKind::Artist => format!(
            "SELECT a.name_key, a.name, NULL, t.path, 0, COUNT(*) OVER (PARTITION BY a.name_key), NULL, NULL, NULL, NULL, 0 FROM track_artist a
             JOIN track t ON t.id=a.track_id WHERE {ACTIVE} AND {PRIMARY} ORDER BY a.name_key, t.path"
        ),
        CollectionKind::Playlist => format!(
            "SELECT CAST(p.id AS TEXT), p.name, NULL, NULL,
                COALESCE(SUM(CASE WHEN i.duration < 1e308 AND {ACTIVE} THEN i.duration ELSE 0 END),0),
                COUNT(CASE WHEN {ACTIVE} THEN t.id END), NULL, NULL, NULL, NULL, 0
             FROM playlist p LEFT JOIN (
                SELECT pt.playlist_id, COALESCE(v.audio_id, pt.track_id) AS track_id
                FROM playlist_track pt LEFT JOIN track_variant v ON v.variant_id=pt.track_id
                GROUP BY pt.playlist_id, COALESCE(v.audio_id, pt.track_id)
             ) pt ON pt.playlist_id=p.id
             LEFT JOIN track_index i ON i.id=pt.track_id LEFT JOIN track t ON t.id=i.id GROUP BY p.id"
        ),
    };
    let text = text.trim().to_lowercase();
    let mut statement = db.prepare(&sql)?;
    let mut rows = statement.query([])?;
    let mut seen = HashSet::new();
    let mut collections = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        if !seen.insert(id.clone()) {
            continue;
        }
        let name: String = row.get(1)?;
        let subtitle: Option<String> = row.get(2)?;
        if !text.is_empty()
            && !format!("{name} {}", subtitle.as_deref().unwrap_or(""))
                .to_lowercase()
                .contains(&text)
        {
            continue;
        }
        let artists = row
            .get::<_, Option<String>>(9)?
            .map(|value| serde_json::from_str::<Vec<String>>(&value))
            .transpose()?
            .filter(|artists| {
                !artists.is_empty() && subtitle.as_deref() == Some(&artists.join(", "))
            });
        let short_release = matches!(kind, CollectionKind::Album)
            && (id.starts_with("single:")
                || row.get::<_, bool>(10)?
                || matches!(
                    catalog::release_title(&name).1,
                    catalog::ReleaseKind::Single | catalog::ReleaseKind::Ep
                ));
        collections.push(CollectionSummary {
            id,
            name,
            subtitle,
            artists,
            short_release,
            cover_path: row.get(3)?,
            duration: row.get(4)?,
            track_count: row.get(5)?,
            year: row.get(6)?,
            added_at: row.get(7)?,
            first_track_id: row.get(8)?,
        });
    }
    Ok(collections)
}

#[tauri::command]
pub async fn library_collections(
    state: State<'_, Library>,
    kind: CollectionKind,
    text: String,
) -> Result<Vec<CollectionSummary>, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        collection_summaries(&connect(&path)?, kind, &text)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)
}

#[tauri::command]
pub async fn library_collection_tracks(
    state: State<'_, Library>,
    kind: CollectionKind,
    id: String,
) -> Result<Vec<Track>, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || read_collection_tracks(&connect(&path)?, kind, &id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(user_error)
}

pub(super) fn read_collection_tracks(db: &Connection, kind: CollectionKind, id: &str) -> LibraryResult<Vec<Track>> {
    let (join, filter, sort) = match kind {
        CollectionKind::Album => ("JOIN track_index i ON i.id=t.id", "i.album_id=?1 AND NOT EXISTS (SELECT 1 FROM track_variant WHERE variant_id=t.id)", "i.disc, i.track_number, i.name_key, t.path"),
        CollectionKind::Artist => (
            "JOIN track_artist a ON a.track_id=t.id JOIN track_index i ON i.id=t.id",
            "a.name_key=?1 AND NOT EXISTS (SELECT 1 FROM track_variant WHERE variant_id=t.id)",
            "COALESCE(json_extract(t.metadata, '$.releaseDate'),
                      printf('%04d', json_extract(t.metadata, '$.year'))) DESC,
             i.album_id, i.disc, i.track_number, i.name_key, t.path",
        ),
        CollectionKind::Playlist => ("JOIN playlist_track p ON COALESCE((SELECT audio_id FROM track_variant WHERE variant_id=p.track_id), p.track_id)=t.id", "p.playlist_id=?1", "p.position"),
    };
    let mut statement = db.prepare(&format!("SELECT t.id, t.path, {}, {} FROM track t {join} WHERE {filter} AND {ACTIVE} ORDER BY {sort}", super::versions::METADATA, super::versions::PLAY_COUNT))?;
    let mut rows = statement.query([id])?;
    let mut tracks = Vec::new();
    let mut seen = HashSet::new();
    while let Some(row) = rows.next()? {
        let track = track_row(row)?;
        if seen.insert(track.id) {
            tracks.push(track);
        }
    }
    Ok(tracks)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn search_queue_starts_at_the_selected_result_and_falls_back_to_the_library() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("library.sqlite3");
        Library::new(database.clone()).unwrap();
        let mut db = connect(&database).unwrap();
        let tx = db.transaction().unwrap();
        for (id, name, active) in [
            (1, "Matching first", true),
            (2, "Matching last", true),
            (3, "Linked result", true),
            (4, "New import", true),
            (5, "Unavailable", false),
        ] {
            let path = format!("/music/{id}.mp3");
            let tags = Tags {
                name: name.into(),
                ..Tags::default()
            };
            tx.execute(
                "INSERT INTO track (id, path, stamp, size, metadata) VALUES (?1, ?2, '', 0, ?3)",
                params![id, path, serde_json::to_string(&tags).unwrap()],
            )
            .unwrap();
            index(&tx, id, &path, &tags).unwrap();
            if active {
                tx.execute("INSERT INTO source VALUES ('/music', ?1)", [id])
                    .unwrap();
            }
        }
        tx.commit().unwrap();

        let search = |text: &str, selected, linked: &[i64]| {
            read_search_queue(
                &db,
                &TrackQuery {
                    sort: Sort::Path,
                    text: text.into(),
                    exclude_playlist: None,
                },
                selected,
                linked.iter().copied(),
            )
            .unwrap()
            .map(|tracks| tracks.into_iter().map(|track| track.id).collect::<Vec<_>>())
        };
        assert_eq!(search("matching", 2, &[2, 3, 3, 5]), Some(vec![2, 3, 1]));
        assert_eq!(search("matching", 4, &[]), Some(vec![4, 1, 2]));
        assert_eq!(search("linked", 3, &[]), Some(vec![3, 4, 1, 2]));
        assert_eq!(search("nothing found", 4, &[]), Some(vec![4, 1, 2, 3]));
        assert_eq!(search("matching", 5, &[]), None);
        db.execute("DELETE FROM source WHERE track_id<>4", [])
            .unwrap();
        assert_eq!(search("new import", 4, &[]), Some(vec![4]));
    }

    #[test]
    fn indexes_untagged_tracks_as_distinct_singles() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library.sqlite3");
        super::super::Library::new(path.clone()).unwrap();
        let mut db = super::super::connect(&path).unwrap();
        let tx = db.transaction().unwrap();
        let tags = Tags {
            name: "Один сингл".into(),
            artists: vec!["Первый".into(), "Второй".into()],
            ..Tags::default()
        };
        for id in [1, 2] {
            let path = format!("/music/{id}.mp3");
            tx.execute(
                "INSERT INTO track (id, path, stamp, size, metadata) VALUES (?1, ?2, '0', 0, ?3)",
                params![id, path, serde_json::to_string(&tags).unwrap()],
            )
            .unwrap();
            tx.execute("INSERT INTO source VALUES ('/music', ?1)", [id])
                .unwrap();
        }
        tx.commit().unwrap();
        migrate_singles(&mut db).unwrap();
        let albums = collection_summaries(&db, CollectionKind::Album, "").unwrap();
        assert_eq!(
            albums
                .iter()
                .map(|album| (album.id.as_str(), album.track_count, album.short_release))
                .collect::<Vec<_>>(),
            [("single:1", 1, true), ("single:2", 1, true)]
        );
    }

    #[test]
    fn marks_short_releases_without_assuming_an_incomplete_album_is_a_single() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("library.sqlite3");
        Library::new(database.clone()).unwrap();
        let mut db = connect(&database).unwrap();
        for (id, album, is_single, is_ep) in [
            (1, "Album", None, false),
            (2, "Tagged EP without a suffix", Some(false), true),
            (3, "Legacy - EP", Some(false), false),
            (4, "Tagged single", Some(true), false),
        ] {
            let tags = Tags {
                name: format!("Track {id}"),
                album: Some(album.into()),
                artists: vec!["Artist, Jr.".into(), "Second".into()],
                is_single,
                is_ep,
                ..Tags::default()
            };
            db.execute(
                "INSERT INTO track (id, path, stamp, size, metadata) VALUES (?1, ?2, '0', 0, ?3)",
                params![
                    id,
                    format!("/music/{id}.mp3"),
                    serde_json::to_string(&tags).unwrap()
                ],
            )
            .unwrap();
            db.execute("INSERT INTO source VALUES ('/music', ?1)", [id])
                .unwrap();
        }
        migrate_singles(&mut db).unwrap();
        let releases = collection_summaries(&db, CollectionKind::Album, "").unwrap();
        let albums: Vec<_> = releases.iter().filter(|item| !item.short_release).collect();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name, "Album");
        assert_eq!(
            albums[0].artists.as_deref().unwrap(),
            ["Artist, Jr.", "Second"]
        );
    }

    #[test]
    fn album_members_share_cover_candidates_in_disc_and_track_order() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("library.sqlite3");
        let library = Library::new(database.clone()).unwrap();
        let mut db = connect(&database).unwrap();
        for (id, number, active) in [(1, 3, true), (2, 2, true), (3, 1, false)] {
            let tags = Tags {
                name: format!("Track {number}"),
                album: Some("Album".into()),
                album_artist: Some("Album Artist".into()),
                artists: vec![format!("Artist {number}")],
                track_number: Some(number),
                ..Tags::default()
            };
            db.execute(
                "INSERT INTO track (id, path, stamp, size, metadata) VALUES (?1, ?2, '0', 0, ?3)",
                params![
                    id,
                    format!("/music/{id}.mp3"),
                    serde_json::to_string(&tags).unwrap()
                ],
            )
            .unwrap();
            if active {
                db.execute("INSERT INTO source VALUES ('/music', ?1)", [id])
                    .unwrap();
            }
        }
        migrate_singles(&mut db).unwrap();

        for path in ["/music/1.mp3", "/music/2.mp3"] {
            assert_eq!(
                library.album_cover_paths(path).unwrap(),
                ["/music/2.mp3", "/music/1.mp3"]
            );
        }
    }

    #[test]
    fn migrates_existing_tracks_and_indexes_literal_cyrillic_and_latin_searches() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE track (id INTEGER PRIMARY KEY, path TEXT UNIQUE, metadata TEXT);
             CREATE TABLE source (root TEXT, track_id INTEGER, PRIMARY KEY(root, track_id));
             CREATE TABLE catalog_state (initialized INTEGER);
             INSERT INTO catalog_state VALUES (1);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        super::super::playlists::migrate(&mut db).unwrap();
        let tags = Tags {
            name: "Трек 10".into(),
            album: Some("Blue \"Moon\"".into()),
            artists: vec!["Би-2".into()],
            ..Tags::default()
        };
        db.execute(
            "INSERT INTO track VALUES (9, '/music/song.mp3', ?1)",
            [serde_json::to_string(&tags).unwrap()],
        )
        .unwrap();
        db.execute("INSERT INTO source VALUES ('/music', 9)", [])
            .unwrap();
        migrate(&mut db).unwrap();
        migrate_added_date(&mut db).unwrap();
        super::super::external::migrate(&mut db).unwrap();
        super::super::versions::migrate(&mut db).unwrap();
        super::super::migrate_play_counts(&mut db).unwrap();

        for text in [
            "РЕК",
            "BLUE \"MOON\"",
            "би",
            "БИ-2",
            "би трек",
            "2 MOON трек",
        ] {
            let query = TrackQuery {
                sort: Sort::Name,
                text: text.into(),
                exclude_playlist: None,
            };
            let page = read_page(&mut db, &query, None).unwrap();
            assert_eq!(
                page.tracks.iter().map(|track| track.id).collect::<Vec<_>>(),
                [9],
                "{text}"
            );
        }
        assert!(read_page(
            &mut db,
            &TrackQuery {
                sort: Sort::Name,
                text: "moon 7".into(),
                exclude_playlist: None
            },
            None
        )
        .unwrap()
        .tracks
        .is_empty());
        let summaries = collection_summaries(&db, CollectionKind::Album, "moon").unwrap();
        assert_eq!(summaries[0].name, "Blue \"Moon\"");
        assert_eq!(summaries[0].cover_path.as_deref(), Some("/music/song.mp3"));
        assert!(collection_summaries(&db, CollectionKind::Playlist, "")
            .unwrap()
            .is_empty());

        let tx = db.transaction().unwrap();
        let tags = Tags {
            name: "Новое имя".into(),
            ..tags
        };
        index(&tx, 9, "/music/song.mp3", &tags).unwrap();
        tx.commit().unwrap();
        let old = TrackQuery {
            sort: Sort::Name,
            text: "трек".into(),
            exclude_playlist: None,
        };
        assert!(read_page(&mut db, &old, None).unwrap().tracks.is_empty());
    }

    #[test]
    fn added_order_pages_tracks_and_uses_the_first_track_added_to_each_album() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite3");
        Library::new(path.clone()).unwrap();
        let mut db = connect(&path).unwrap();
        let tx = db.transaction().unwrap();
        for id in 1..=55 {
            let path = format!("/music/{id}.mp3");
            let album = match id {
                1 => "Старый",
                55 => "Новый",
                _ => "Первый",
            };
            let tags = Tags {
                album: Some(album.into()),
                ..Tags::default()
            };
            let added_at = match id {
                1 => None,
                55 => Some(2000),
                _ => Some(1000),
            };
            tx.execute(
                "INSERT INTO track (id, path, stamp, size, metadata, added_at)
                 VALUES (?1, ?2, '', 0, ?3, ?4)",
                params![id, path, serde_json::to_string(&tags).unwrap(), added_at],
            )
            .unwrap();
            index(&tx, id, &path, &tags).unwrap();
            tx.execute("INSERT INTO source VALUES ('/music', ?1)", [id])
                .unwrap();
        }
        tx.commit().unwrap();

        let query = TrackQuery {
            sort: Sort::Added,
            text: String::new(),
            exclude_playlist: None,
        };
        let first = read_page(&mut db, &query, None).unwrap();
        let second = read_page(&mut db, &query, first.next_cursor).unwrap();
        let ids = first
            .tracks
            .into_iter()
            .chain(second.tracks)
            .map(|track| track.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, (1..=55).rev().collect::<Vec<_>>());

        let (_, selected) = read_queue(&db, &query, QueueTarget::Id { id: 7 })
            .unwrap()
            .unwrap();
        assert_eq!(selected, 48);
        let albums = collection_summaries(&db, CollectionKind::Album, "").unwrap();
        let added = |name: &str| {
            albums
                .iter()
                .find(|album| album.name == name)
                .unwrap()
                .added_at
        };
        assert_eq!(added("Новый"), Some(2000));
        assert_eq!(added("Первый"), Some(1000));
        assert_eq!(added("Старый"), None);
    }

    #[test]
    fn pages_fifty_thousand_tracks_and_plays_the_full_filtered_selection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite3");
        Library::new(path.clone()).unwrap();
        let mut db = connect(&path).unwrap();
        let tx = db.transaction().unwrap();
        {
            let mut track = tx
                .prepare("INSERT INTO track (id, path, stamp, size, metadata) VALUES (?1, ?2, '0', 0, ?3)")
                .unwrap();
            let mut source = tx
                .prepare("INSERT INTO source VALUES ('/music', ?1)")
                .unwrap();
            for id in 1..=50_000 {
                let path = format!("/music/{id}.mp3");
                let tags = Tags {
                    name: if id % 2 == 0 {
                        format!("Трек {id}")
                    } else {
                        format!("Track {id}")
                    },
                    ..Tags::default()
                };
                track
                    .execute(params![id, path, serde_json::to_string(&tags).unwrap()])
                    .unwrap();
                index(&tx, id, &path, &tags).unwrap();
                source.execute([id]).unwrap();
            }
        }
        tx.commit().unwrap();

        let query = TrackQuery {
            sort: Sort::Name,
            text: "ТРЕК".into(),
            exclude_playlist: None,
        };
        let started = Instant::now();
        let first = read_page(&mut db, &query, None).unwrap();
        let second = read_page(&mut db, &query, first.next_cursor).unwrap();
        let page_time = started.elapsed();
        assert_eq!(first.tracks.len(), PAGE_SIZE);
        assert_eq!(first.tracks.first().unwrap().id, 2);
        assert_eq!(first.tracks.last().unwrap().id, 96);
        assert_eq!(second.tracks.first().unwrap().id, 98);
        let started = Instant::now();
        let (queue, selected) = read_queue(&db, &query, QueueTarget::Id { id: 100 })
            .unwrap()
            .unwrap();
        let queue_time = started.elapsed();
        assert_eq!(queue.len(), 25_000);
        assert_eq!(selected, 49);

        db.execute("UPDATE catalog_state SET revision=revision+1", [])
            .unwrap();
        let refreshed = read_page(&mut db, &query, second.next_cursor).unwrap();
        assert_eq!(refreshed.tracks.first().unwrap().id, 2);
        let latin = TrackQuery {
            sort: Sort::Path,
            text: "TRACK 999".into(),
            exclude_playlist: None,
        };
        assert_eq!(read_page(&mut db, &latin, None).unwrap().tracks[0].id, 999);
        eprintln!(
            "50,000 tracks: two filtered pages {page_time:?}; 25,000-track queue {queue_time:?}"
        );
    }
}
