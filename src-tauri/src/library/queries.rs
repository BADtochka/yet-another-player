use std::collections::HashSet;

use icu_collator::{
    options::{CollatorOptions, Strength},
    preferences::CollationNumericOrdering,
    Collator, CollatorPreferences,
};
use icu_locale_core::locale;
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{connect, user_error, Library, LibraryResult, Tags, Track};
use crate::audio::{Action, AudioEngine, Snapshot};

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

thread_local! {
    static COLLATOR: Result<icu_collator::CollatorBorrowed<'static>, String> = {
        let mut preferences: CollatorPreferences = locale!("ru").into();
        preferences.numeric_ordering = Some(CollationNumericOrdering::True);
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Primary);
        Collator::try_new(preferences, options).map_err(|error| error.to_string())
    };
}

pub(super) fn index(db: &Transaction<'_>, id: i64, path: &str, tags: &Tags) -> LibraryResult<()> {
    let artists = tags.artists.join(", ");
    let album = tags
        .album
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let album_artist = tags
        .album_artist
        .as_deref()
        .map(str::trim)
        .filter(|artist| !artist.is_empty())
        .unwrap_or(&artists);
    let album_id = album
        .map(|name| serde_json::to_string(&[name, album_artist]))
        .transpose()?;
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
const PAGE_SIZE: usize = 48;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sort {
    Name,
    Artist,
    Duration,
    Path,
}

impl Sort {
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Name => &["i.name_key", "i.path_key", "i.id"],
            Self::Artist => &["i.artist_key", "i.name_key", "i.path_key", "i.id"],
            Self::Duration => &["i.duration", "i.name_key", "i.path_key", "i.id"],
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
             WHERE {ACTIVE} AND {predicate} {cursor}
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
    let metadata: String = row.get(2)?;
    Ok(Track {
        id: row.get(0)?,
        path: row.get(1)?,
        tags: serde_json::from_str(&metadata)?,
    })
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
    let (sql, term, short, phrase) = query.select("t.id, t.path, t.metadata", after.is_some())?;
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

fn read_queue(
    db: &Connection,
    query: &TrackQuery,
    target: QueueTarget,
) -> LibraryResult<Option<(Vec<crate::audio::Track>, usize)>> {
    let (sql, term, short, phrase) = query.select("t.id, t.path, i.gain", false)?;
    let mut statement = db.prepare(&sql)?;
    let mut rows = statement.query(params![
        term,
        None::<i64>,
        -1,
        query.exclude_playlist,
        short,
        phrase
    ])?;
    let mut queue = Vec::new();
    let mut selected = None;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let path: String = row.get(1)?;
        let matches = match &target {
            QueueTarget::Id { id: target } => *target == id,
            QueueTarget::Path { path: target } => *target == path,
            QueueTarget::First => queue.is_empty(),
            QueueTarget::Last => true,
        };
        if matches {
            selected = Some(queue.len());
        }
        queue.push(crate::audio::Track::local(path, row.get(2)?));
    }
    Ok(selected.map(|index| (queue, index)))
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
    state: State<'_, Library>,
    audio: State<'_, AudioEngine>,
    query: TrackQuery,
    target: QueueTarget,
    position: f64,
    playing: bool,
) -> Result<Option<Snapshot>, String> {
    let path = state.database.clone();
    let request = audio.reserve_queue();
    let queue =
        tauri::async_runtime::spawn_blocking(move || read_queue(&connect(&path)?, &query, target))
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
        let mut statement = db.prepare("SELECT id, path, metadata FROM track WHERE path=?1")?;
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
    cover_path: Option<String>,
    duration: f64,
    track_count: i64,
    year: Option<u16>,
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
                    COUNT(*) OVER album, MAX(json_extract(t.metadata, '$.year')) OVER album
             FROM track_index i JOIN track t ON t.id=i.id
             WHERE i.album_id IS NOT NULL AND {ACTIVE}
             WINDOW album AS (PARTITION BY i.album_id)
             ORDER BY i.album_id, i.disc, i.track_number, i.name_key, t.path"
        ),
        CollectionKind::Artist => format!(
            "SELECT a.name_key, a.name, NULL, NULL, 0, COUNT(*) OVER (PARTITION BY a.name_key), NULL FROM track_artist a
             JOIN track t ON t.id=a.track_id WHERE {ACTIVE} ORDER BY a.name_key, t.path"
        ),
        CollectionKind::Playlist => format!(
            "SELECT CAST(p.id AS TEXT), p.name, NULL, NULL,
                COALESCE(SUM(CASE WHEN i.duration < 1e308 AND {ACTIVE} THEN i.duration ELSE 0 END),0),
                COUNT(CASE WHEN {ACTIVE} THEN t.id END), NULL
             FROM playlist p LEFT JOIN playlist_track pt ON pt.playlist_id=p.id
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
        collections.push(CollectionSummary {
            id,
            name,
            subtitle,
            cover_path: row.get(3)?,
            duration: row.get(4)?,
            track_count: row.get(5)?,
            year: row.get(6)?,
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
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<Vec<Track>> {
        let db = connect(&path)?;
        let (join, filter, sort) = match kind {
            CollectionKind::Album => ("JOIN track_index i ON i.id=t.id", "i.album_id=?1", "i.disc, i.track_number, i.name_key, t.path"),
            CollectionKind::Artist => ("JOIN track_artist a ON a.track_id=t.id", "a.name_key=?1", "t.path"),
            CollectionKind::Playlist => ("JOIN playlist_track p ON p.track_id=t.id", "p.playlist_id=?1", "p.position"),
        };
        let mut statement = db.prepare(&format!("SELECT t.id, t.path, t.metadata FROM track t {join} WHERE {filter} AND {ACTIVE} ORDER BY {sort}"))?;
        let mut rows = statement.query([id])?;
        let mut tracks = Vec::new();
        while let Some(row) = rows.next()? {
            tracks.push(track_row(row)?);
        }
        Ok(tracks)
    }).await.map_err(|error| error.to_string())?.map_err(user_error)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

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
    fn pages_fifty_thousand_tracks_and_plays_the_full_filtered_selection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite3");
        Library::new(path.clone()).unwrap();
        let mut db = connect(&path).unwrap();
        let tx = db.transaction().unwrap();
        {
            let mut track = tx
                .prepare("INSERT INTO track VALUES (?1, ?2, '0', 0, ?3)")
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
