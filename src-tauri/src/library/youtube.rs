use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use super::{connect, user_error, Library, LibraryResult, Tags, Track};
use crate::{external_tools::valid_youtube_id, MusicFolders};

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct DownloadTags {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<u16>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

impl DownloadTags {
    pub(super) fn apply(&self, tags: &mut Tags) {
        tags.name.clone_from(&self.title);
        if !self.artists.is_empty() {
            tags.artists.clone_from(&self.artists);
        }
        if self.album.is_some() {
            tags.album.clone_from(&self.album);
        }
        if self.album_artist.is_some() {
            tags.album_artist.clone_from(&self.album_artist);
        }
        tags.year = self.year.or(tags.year);
        tags.track_number = self.track_number.or(tags.track_number);
        tags.disc_number = self.disc_number.or(tags.disc_number);
    }
}

pub(crate) struct Download<'a> {
    pub video_id: &'a str,
    pub tags: &'a DownloadTags,
    pub cover: Option<&'a [u8]>,
}

pub(super) fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE youtube_download (
            track_id INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
            video_id TEXT NOT NULL,
            metadata TEXT NOT NULL,
            cover BLOB
        );
        CREATE INDEX youtube_download_video ON youtube_download(video_id);
        PRAGMA user_version = 4;",
    )?;
    tx.commit()
}

pub(super) fn record(tx: &Transaction<'_>, id: i64, download: Download<'_>) -> LibraryResult<()> {
    tx.execute(
        "INSERT INTO youtube_download VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(track_id) DO UPDATE SET video_id=excluded.video_id, metadata=excluded.metadata,
            cover=COALESCE(excluded.cover, youtube_download.cover)",
        params![
            id,
            download.video_id,
            serde_json::to_string(download.tags)?,
            download.cover,
        ],
    )?;
    Ok(())
}

pub(super) fn stored_tags(tx: &Transaction<'_>, path: &str) -> LibraryResult<Option<DownloadTags>> {
    let metadata: Option<String> = tx
        .query_row(
            "SELECT y.metadata FROM youtube_download y JOIN track t ON t.id = y.track_id
             WHERE t.path = ?1",
            [path],
            |row| row.get(0),
        )
        .optional()?;
    Ok(metadata.and_then(|value| serde_json::from_str(&value).ok()))
}

impl Library {
    pub(crate) fn active_youtube_tracks(
        &self,
        ids: &[String],
        roots: &[PathBuf],
    ) -> LibraryResult<HashMap<String, Track>> {
        let db = connect(&self.database)?;
        let mut statement = db.prepare(
            "SELECT t.id, t.path, t.metadata FROM youtube_download y
             JOIN track t ON t.id=y.track_id
             WHERE y.video_id=?1 AND EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
             ORDER BY t.id",
        )?;
        let mut found = HashMap::new();
        for id in ids {
            let mut rows = statement.query([id])?;
            while let Some(row) = rows.next()? {
                let path: String = row.get(1)?;
                if available_track(&path, roots) {
                    found.insert(id.clone(), linked_track(row, path)?);
                    break;
                }
            }
        }
        if found.len() < ids.len() {
            // Older versions only kept the video ID in the filename.
            let mut legacy = db.prepare(
                "SELECT t.id, t.path, t.metadata FROM track t
                 WHERE instr(t.path, ' [')>0 AND EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
                 ORDER BY t.id",
            )?;
            let mut rows = legacy.query([])?;
            while let Some(row) = rows.next()? {
                let path: String = row.get(1)?;
                let Some(id) = legacy_id(Path::new(&path)) else {
                    continue;
                };
                if ids.iter().any(|candidate| candidate == id)
                    && !found.contains_key(id)
                    && available_track(&path, roots)
                {
                    found.insert(id.to_owned(), linked_track(row, path)?);
                }
            }
        }
        Ok(found)
    }

    pub(crate) fn youtube_paths(&self, video_id: &str) -> LibraryResult<Vec<PathBuf>> {
        let db = connect(&self.database)?;
        let mut statement = db.prepare(
            "SELECT t.path FROM youtube_download y JOIN track t ON t.id = y.track_id
             WHERE y.video_id = ?1 ORDER BY t.id",
        )?;
        let paths = statement.query_map([video_id], |row| row.get::<_, String>(0))?;
        Ok(paths
            .map(|path| path.map(PathBuf::from))
            .collect::<Result<_, _>>()?)
    }

    pub(crate) fn youtube_cover(&self, path: &Path) -> Option<Vec<u8>> {
        connect(&self.database)
            .ok()?
            .query_row(
                "SELECT y.cover FROM youtube_download y JOIN track t ON t.id = y.track_id
                 WHERE t.path = ?1",
                [path.to_str()?],
                |row| row.get(0),
            )
            .ok()?
    }
}

fn legacy_id(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let (_, id) = stem.strip_suffix(']')?.rsplit_once(" [")?;
    valid_youtube_id(id).then_some(id)
}

fn available_track(path: &str, roots: &[PathBuf]) -> bool {
    fs::canonicalize(path).is_ok_and(|canonical| {
        roots.iter().any(|root| canonical.starts_with(root))
            && fs::metadata(&canonical).is_ok_and(|file| file.is_file() && file.len() > 0)
    })
}

fn linked_track(row: &rusqlite::Row<'_>, path: String) -> LibraryResult<Track> {
    let metadata: String = row.get(2)?;
    Ok(Track {
        id: row.get(0)?,
        path,
        tags: serde_json::from_str(&metadata)?,
    })
}

#[tauri::command]
pub(crate) async fn library_youtube_tracks(
    app: tauri::AppHandle,
    folders: State<'_, MusicFolders>,
    ids: Vec<String>,
) -> Result<HashMap<String, Track>, String> {
    if ids.len() > 24 || !ids.iter().all(|id| valid_youtube_id(id)) {
        return Err("Некорректные идентификаторы YouTube".into());
    }
    let roots = folders
        .canonical
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Library>().active_youtube_tracks(&ids, &roots)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)
}
