use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use super::{connect, user_error, Library, LibraryResult, Tags, Track};
use crate::{external_tools::valid_video_id, MusicFolders};

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct DownloadTags {
    pub title: String,
    #[serde(default)]
    pub is_clip: bool,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<u16>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

impl DownloadTags {
    pub(super) fn normalize(&mut self) {
        self.title =
            crate::external_tools::music_metadata::normalize(&self.title, &mut self.artists);
    }

    pub(super) fn apply(&self, tags: &mut Tags) {
        tags.name.clone_from(&self.title);
        tags.fallback_title = false;
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

pub(super) fn migrate_source_names(db: &mut Connection) -> LibraryResult<()> {
    let tx = db.transaction()?;
    let old_table = concat!("you", "tube_download");
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [old_table],
        |row| row.get(0),
    )?;
    if exists {
        tx.execute_batch(&format!(
            "ALTER TABLE {old_table} RENAME TO external_download;
             DROP INDEX IF EXISTS {old_table}_video;
             CREATE INDEX external_download_video ON external_download(video_id);"
        ))?;
        let old_key = concat!("$.you", "tubeVideoId");
        tx.execute("UPDATE track SET metadata=json_remove(json_set(metadata, '$.externalVideoId', json_extract(metadata, ?1)), ?1)
                    WHERE json_type(metadata, ?1) IS NOT NULL", [old_key])?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE external_download (
            track_id INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
            video_id TEXT NOT NULL,
            metadata TEXT NOT NULL,
            cover BLOB
        );
        CREATE INDEX external_download_video ON external_download(video_id);
        PRAGMA user_version = 4;",
    )?;
    tx.commit()
}

pub(super) fn record(tx: &Transaction<'_>, id: i64, download: Download<'_>) -> LibraryResult<()> {
    tx.execute(
        "INSERT INTO external_download VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(track_id) DO UPDATE SET video_id=excluded.video_id, metadata=excluded.metadata,
            cover=COALESCE(excluded.cover, external_download.cover)",
        params![
            id,
            download.video_id,
            serde_json::to_string(download.tags)?,
            download.cover,
        ],
    )?;
    Ok(())
}

pub(super) fn stored_tags(
    tx: &Transaction<'_>,
    path: &str,
) -> LibraryResult<Option<(String, DownloadTags)>> {
    let entry: Option<(String, String)> = tx
        .query_row(
            "SELECT y.video_id, y.metadata FROM external_download y JOIN track t ON t.id = y.track_id
             WHERE t.path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(entry.and_then(|(id, value)| serde_json::from_str(&value).ok().map(|tags| (id, tags))))
}

impl Library {
    pub(crate) fn active_external_tracks(
        &self,
        ids: &[String],
        roots: &[PathBuf],
    ) -> LibraryResult<HashMap<String, Track>> {
        let db = connect(&self.database)?;
        let mut statement = db.prepare(&format!(
            "SELECT t.id, t.path, {} FROM external_download y
             LEFT JOIN track_variant r ON r.variant_id=y.track_id
             JOIN track t ON t.id=COALESCE(r.audio_id, y.track_id)
             WHERE y.video_id=?1 AND EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
             ORDER BY t.id",
            super::versions::METADATA
        ))?;
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

    pub(crate) fn external_paths(&self, video_id: &str) -> LibraryResult<Vec<PathBuf>> {
        let db = connect(&self.database)?;
        let mut statement = db.prepare(
            "SELECT t.path FROM external_download y JOIN track t ON t.id = y.track_id
             WHERE y.video_id = ?1 ORDER BY t.id",
        )?;
        let paths = statement.query_map([video_id], |row| row.get::<_, String>(0))?;
        Ok(paths
            .map(|path| path.map(PathBuf::from))
            .collect::<Result<_, _>>()?)
    }

    pub(crate) fn external_cover(&self, path: &Path) -> Option<Vec<u8>> {
        connect(&self.database)
            .ok()?
            .query_row(
                "SELECT y.cover FROM external_download y JOIN track t ON t.id = y.track_id
                 WHERE t.path = ?1",
                [path.to_str()?],
                |row| row.get(0),
            )
            .ok()?
    }
}

pub(super) fn legacy_id(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let (_, id) = stem.strip_suffix(']')?.rsplit_once(" [")?;
    valid_video_id(id).then_some(id)
}

fn available_track(path: &str, roots: &[PathBuf]) -> bool {
    fs::canonicalize(path).is_ok_and(|canonical| {
        roots.iter().any(|root| canonical.starts_with(root))
            && fs::metadata(&canonical).is_ok_and(|file| file.is_file() && file.len() > 0)
    })
}

fn linked_track(row: &rusqlite::Row<'_>, path: String) -> LibraryResult<Track> {
    Track::from_metadata(row.get(0)?, path, &row.get::<_, String>(2)?)
}

#[tauri::command]
pub(crate) async fn library_external_tracks(
    app: tauri::AppHandle,
    folders: State<'_, MusicFolders>,
    ids: Vec<String>,
) -> Result<HashMap<String, Track>, String> {
    if ids.len() > 24 || !ids.iter().all(|id| valid_video_id(id)) {
        return Err("Некорректные идентификаторы видео".into());
    }
    let roots = folders
        .canonical
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Library>().active_external_tracks(&ids, &roots)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)
}
