use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Response, AppHandle, Emitter, Manager, State};

use super::{connect, queries, versions, Library, LibraryResult, Track};
use crate::{
    audio::{library_track_path, Action, AudioEngine},
    cover, MusicFolders,
};

const DIRECTORY_PREFIX: &str = ".yap-trash-";
const MIN_DAYS: u32 = 1;
const MAX_DAYS: u32 = 365;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Trash {
    retention_days: u32,
    entries: Vec<Entry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: i64,
    track: Track,
    removed_at: i64,
    expires_at: i64,
}

pub(super) fn migrate_entries(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "ALTER TABLE track_trash RENAME TO track_trash_v13;
         CREATE TABLE track_trash (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            track_id INTEGER NOT NULL REFERENCES track(id),
            removed_at INTEGER NOT NULL,
            directory TEXT NOT NULL,
            roots TEXT NOT NULL,
            stamp TEXT NOT NULL,
            size INTEGER NOT NULL,
            metadata TEXT NOT NULL
         );
         INSERT INTO track_trash (id, track_id, removed_at, directory, roots, stamp, size, metadata)
            SELECT b.track_id, b.track_id, b.removed_at, b.directory, b.roots, t.stamp, t.size, t.metadata
            FROM track_trash_v13 b JOIN track t ON t.id=b.track_id;
         DROP TABLE track_trash_v13;
         CREATE INDEX track_trash_track ON track_trash(track_id);
         PRAGMA user_version=14;",
    )?;
    tx.commit()
}

fn retention(db: &Connection) -> rusqlite::Result<u32> {
    db.query_row("SELECT retention_days FROM trash_settings", [], |row| {
        row.get(0)
    })
}

fn payload(original: &Path, directory: &Path) -> io::Result<PathBuf> {
    let parent = original
        .parent()
        .ok_or_else(|| io::Error::other("Некорректный путь трека."))?;
    if directory.parent() != Some(parent)
        || !directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(DIRECTORY_PREFIX))
        || fs::canonicalize(parent)? != parent
    {
        return Err(io::Error::other("Некорректный путь корзины."));
    }
    match fs::symlink_metadata(directory) {
        Ok(info) if info.is_dir() => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => return Err(io::Error::other("Папка корзины была заменена.")),
        Err(error) => return Err(error),
    }
    Ok(directory.join("audio"))
}

pub(super) fn has_interrupted_move(db: &Connection, path: &Path) -> LibraryResult<bool> {
    let mut statement = db.prepare_cached(
        "SELECT b.directory FROM track t JOIN track_trash b ON b.track_id=t.id
         WHERE t.path=?1 AND NOT EXISTS (SELECT 1 FROM source WHERE track_id=t.id)",
    )?;
    let mut rows = statement.query([path.to_str()])?;
    while let Some(row) = rows.next()? {
        let directory: String = row.get(0)?;
        if matches!(
            payload(path, Path::new(&directory)).and_then(|stored| stored.try_exists()),
            Ok(false)
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn restore_catalog(
    tx: &Transaction<'_>,
    entry_id: i64,
    track_id: i64,
    roots: &[String],
) -> LibraryResult<()> {
    tx.execute("DELETE FROM track_trash WHERE id=?1", [entry_id])?;
    for root in roots {
        tx.execute(
            "INSERT OR IGNORE INTO source (root, track_id) VALUES (?1, ?2)",
            params![root, track_id],
        )?;
    }
    versions::rebuild(tx)?;
    tx.execute("UPDATE catalog_state SET revision=revision+1", [])?;
    Ok(())
}

fn stage_track(db: &mut Connection, path: &Path) -> LibraryResult<()> {
    let path_str = path.to_str().ok_or("Некорректное имя файла.")?;
    let parent = path.parent().ok_or("Файл трека недоступен.")?;
    let id: i64 = db.query_row(
        "SELECT id FROM track WHERE path=?1 AND EXISTS (SELECT 1 FROM source WHERE track_id=track.id)",
        [path_str], |row| row.get(0),
    ).map_err(|_| "Трек больше не находится в медиатеке.")?;
    let roots = {
        let mut statement = db.prepare("SELECT root FROM source WHERE track_id=?1")?;
        let rows = statement.query_map([id], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let directory = tempfile::Builder::new()
        .prefix(DIRECTORY_PREFIX)
        .tempdir_in(parent)?;
    let stored = payload(path, directory.path())?;
    let location = directory
        .path()
        .to_str()
        .ok_or("Некорректное имя папки корзины.")?;
    let tx = db.transaction()?;
    // Commit the location before moving the file so an interruption leaves a recoverable record.
    let entry_id: i64 = tx.query_row(
        "INSERT INTO track_trash (track_id, removed_at, directory, roots, stamp, size, metadata)
         SELECT id, unixepoch(), ?2, ?3, stamp, size, metadata FROM track WHERE id=?1 RETURNING id",
        params![id, location, serde_json::to_string(&roots)?],
        |row| row.get(0),
    )?;
    tx.execute("DELETE FROM source WHERE track_id=?1", [id])?;
    versions::rebuild(&tx)?;
    tx.execute("UPDATE catalog_state SET revision=revision+1", [])?;
    tx.commit()?;
    let directory = directory.keep();
    if let Err(error) = fs::rename(path, stored) {
        let tx = db.transaction()?;
        restore_catalog(&tx, entry_id, id, &roots)?;
        tx.commit()?;
        let _ = fs::remove_dir(directory);
        return Err(error.into());
    }
    Ok(())
}

fn restore_file(source: &Path, destination: &Path) -> io::Result<()> {
    let info = fs::symlink_metadata(source)?;
    if !info.is_file() {
        return Err(io::Error::other("Файл корзины был заменён."));
    }
    if let Err(error) = fs::hard_link(source, destination) {
        if error.kind() == io::ErrorKind::AlreadyExists {
            return Err(io::Error::other("По прежнему адресу уже есть файл."));
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let copy = (|| {
            let mut input = fs::File::open(source)?;
            io::copy(&mut input, &mut output)?;
            if let Ok(modified) = info.modified() {
                output.set_times(fs::FileTimes::new().set_modified(modified))?;
            }
            output.set_permissions(info.permissions())?;
            output.sync_all()
        })();
        if let Err(error) = copy {
            drop(output);
            let _ = fs::remove_file(destination);
            return Err(error);
        }
    }
    if let Err(error) = fs::remove_file(source) {
        let _ = fs::remove_file(destination);
        return Err(error);
    }
    Ok(())
}

fn remove_expired(db: &Connection) -> LibraryResult<()> {
    let seconds = i64::from(retention(db)?) * 86_400;
    purge(db, None, Some(seconds))?;
    Ok(())
}

fn purge(db: &Connection, id: Option<i64>, expiry: Option<i64>) -> LibraryResult<usize> {
    let rows = {
        let mut stmt = db.prepare(
            "SELECT b.id, t.path, b.directory, EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
             FROM track_trash b JOIN track t ON t.id=b.track_id
             WHERE (?1 IS NULL OR b.id=?1) AND (?2 IS NULL OR b.removed_at <= unixepoch()-?2)",
        )?;
        let rows = stmt.query_map(params![id, expiry], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut failed = 0;
    for (id, path, directory, active) in rows {
        let directory = Path::new(&directory);
        let path = Path::new(&path);
        let result = (|| -> io::Result<bool> {
            let stored = payload(path, directory)?;
            match fs::symlink_metadata(&stored) {
                Ok(info) if info.is_file() => fs::remove_file(&stored)?,
                Ok(_) => return Err(io::Error::other("Файл корзины был заменён.")),
                // A move or restore may have been interrupted. Keep the original recoverable.
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        && !active
                        && path.try_exists()? =>
                {
                    return Ok(false)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let _ = fs::remove_dir(directory);
            Ok(true)
        })();
        match result {
            Ok(true) => {
                db.execute("DELETE FROM track_trash WHERE id=?1", [id])?;
            }
            Ok(false) => failed += 1,
            Err(error) => {
                failed += 1;
                eprintln!("Trash checkout {id}: {error}");
            }
        }
    }
    Ok(failed)
}

async fn work<T: Send + 'static>(
    library: &Library,
    task: impl FnOnce(&mut Connection) -> LibraryResult<T> + Send + 'static,
) -> Result<T, String> {
    let writer = library.lock_catalog_writer().await;
    let database = library.database_path().to_owned();
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<T> {
        let _writer = writer;
        task(&mut connect(&database)?)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

pub(crate) fn start_maintenance(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut clock = tokio::time::interval(Duration::from_secs(3600));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            clock.tick().await;
            if let Err(error) = work(&app.state::<Library>(), |db| remove_expired(db)).await {
                eprintln!("Track trash: {error}");
            }
        }
    });
}

#[tauri::command]
pub(crate) async fn library_trash_list(state: State<'_, Library>) -> Result<Trash, String> {
    work(&state, |db| {
        remove_expired(db)?;
        let retention_days = retention(db)?;
        let mut statement = db.prepare(
            "SELECT b.id, t.id, t.path, b.metadata, t.play_count, b.removed_at FROM track_trash b
             JOIN track t ON t.id=b.track_id ORDER BY b.removed_at DESC, b.id DESC",
        )?;
        let mut rows = statement.query([])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let removed_at: i64 = row.get(5)?;
            entries.push(Entry {
                id: row.get(0)?,
                track: Track::from_metadata(
                    row.get(1)?,
                    row.get(2)?,
                    &row.get::<_, String>(3)?,
                    row.get(4)?,
                )?,
                removed_at,
                expires_at: removed_at + i64::from(retention_days) * 86_400,
            });
        }
        Ok(Trash {
            retention_days,
            entries,
        })
    })
    .await
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum PurgeTarget {
    Entry { id: i64 },
    All,
}

#[tauri::command]
pub(crate) async fn library_purge_trash(
    state: State<'_, Library>,
    target: PurgeTarget,
) -> Result<(), String> {
    let id = match target {
        PurgeTarget::Entry { id } => Some(id),
        PurgeTarget::All => None,
    };
    if id.is_some_and(|id| id <= 0) {
        return Err("Некорректная запись корзины.".into());
    }
    work(&state, move |db| {
        let failed = purge(db, id, None)?;
        if failed > 0 {
            return Err(format!("Не удалось удалить записей: {failed}.").into());
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub(crate) async fn library_trash_cover(
    state: State<'_, Library>,
    id: i64,
) -> Result<Response, String> {
    let database = state.database_path().to_owned();
    let result = tauri::async_runtime::spawn_blocking(move || -> LibraryResult<Vec<u8>> {
        let db = connect(&database)?;
        let paths: Option<(String, String)> = db.query_row(
            "SELECT t.path, b.directory FROM track_trash b JOIN track t ON t.id=b.track_id WHERE b.id=?1",
            [id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((path, directory)) = paths else { return Ok(Vec::new()) };
        let path = Path::new(&path);
        if let Ok(stored) = payload(path, Path::new(&directory)) {
            if fs::symlink_metadata(&stored).is_ok_and(|info| info.is_file()) {
                if let Some(bytes) = cover::folder_cover(path)
                    .or_else(|| cover::extract_cover(&stored).map(|(bytes, _)| bytes))
                {
                    return Ok(bytes);
                }
            }
        }
        Ok(db.query_row(
            "SELECT COALESCE(art.bytes, y.cover) FROM track_trash b
             JOIN track t ON t.id=b.track_id
             LEFT JOIN resolver_artwork art ON art.url=json_extract(b.metadata, '$.coverUrl')
             LEFT JOIN external_download y ON y.track_id=t.id AND t.stamp=b.stamp AND t.size=b.size
             WHERE b.id=?1",
            [id], |row| row.get::<_, Option<Vec<u8>>>(0),
        ).optional()?.flatten().unwrap_or_default())
    }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
    Ok(Response::new(result))
}

#[tauri::command]
pub(crate) async fn library_set_trash_retention(
    state: State<'_, Library>,
    days: u32,
) -> Result<(), String> {
    if !(MIN_DAYS..=MAX_DAYS).contains(&days) {
        return Err("Выберите срок от 1 до 365 дней.".into());
    }
    work(&state, move |db| {
        db.execute("UPDATE trash_settings SET retention_days=?1", [days])?;
        remove_expired(db)
    })
    .await
}

#[tauri::command]
pub(crate) async fn library_trash_track(
    app: AppHandle,
    state: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<(), String> {
    let roots = folders.canonical.clone();
    let removed = work(&state, move |db| {
        let path = library_track_path(&path, &roots)?;
        stage_track(db, &path)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await?;
    let _ = app
        .state::<AudioEngine>()
        .control(Action::RemovePath { path: removed })
        .await;
    let _ = app.emit("library:metadata", ());
    Ok(())
}

fn restore_entry(db: &mut Connection, id: i64, configured: &[String]) -> LibraryResult<()> {
    let tx = db.transaction()?;
    let (track_id, path, directory, roots, active): (i64, String, String, String, bool) = tx.query_row(
        "SELECT t.id, t.path, b.directory, b.roots, EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
         FROM track_trash b JOIN track t ON t.id=b.track_id WHERE b.id=?1",
        [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).optional()?.ok_or("Трека больше нет в корзине.")?;
    let path = Path::new(&path);
    let directory = Path::new(&directory);
    let stored = payload(path, directory)?;
    let mut roots: Vec<String> = serde_json::from_str(&roots)?;
    roots.retain(|root| configured.contains(root));
    if roots.is_empty() {
        return Err("Сначала верните исходную папку в медиатеку.".into());
    }
    let metadata: String = tx.query_row(
        "UPDATE track SET (stamp, size, metadata) = (SELECT stamp, size, metadata FROM track_trash WHERE id=?1)
         WHERE id=?2 RETURNING metadata",
        params![id, track_id],
        |row| row.get(0),
    )?;
    queries::index(
        &tx,
        track_id,
        path.to_str().ok_or("Некорректное имя файла.")?,
        &serde_json::from_str(&metadata)?,
    )?;
    let moved = stored.try_exists()?;
    if moved {
        restore_file(&stored, path)?;
    } else if active || !fs::symlink_metadata(path).is_ok_and(|info| info.is_file()) {
        return Err("Файл трека недоступен.".into());
    }
    if let Err(error) = restore_catalog(&tx, id, track_id, &roots).and_then(|()| Ok(tx.commit()?)) {
        if moved {
            let _ = restore_file(path, &stored);
        }
        return Err(error);
    }
    let _ = fs::remove_dir(directory);
    Ok(())
}

#[tauri::command]
pub(crate) async fn library_restore_track(
    app: AppHandle,
    state: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    id: i64,
) -> Result<(), String> {
    let configured = folders
        .configured
        .lock()
        .map_err(|_| "Медиатека недоступна.")?
        .clone();
    work(&state, move |db| restore_entry(db, id, &configured)).await?;
    let _ = app.emit("library:metadata", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_preserves_original_until_expiry_and_never_deletes_a_replacement_file() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().canonicalize().unwrap().join("track.flac");
        fs::write(&original, b"original audio").unwrap();
        let library = Library::new(temp.path().join("library.sqlite3")).unwrap();
        let mut db = connect(library.database_path()).unwrap();
        db.execute(
            "INSERT INTO track (id,path,stamp,size,metadata) VALUES (1,?1,'',14,'{}')",
            [original.to_str().unwrap()],
        )
        .unwrap();
        db.execute("INSERT INTO source VALUES ('library',1)", [])
            .unwrap();
        stage_track(&mut db, &original).unwrap();
        assert!(!original.exists());
        let directory: String = db
            .query_row(
                "SELECT directory FROM track_trash WHERE track_id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let saved = payload(&original, Path::new(&directory)).unwrap();
        remove_expired(&db).unwrap();
        assert_eq!(fs::read(&saved).unwrap(), b"original audio");

        fs::write(&original, b"replacement").unwrap();
        assert!(restore_file(&saved, &original).is_err());
        db.execute("UPDATE track_trash SET removed_at=unixepoch()-31*86400", [])
            .unwrap();
        remove_expired(&db).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"replacement");
        assert!(!saved.exists());
    }

    #[test]
    fn manual_purge_removes_selected_payloads_but_preserves_replacements() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let library = Library::new(temp.path().join("library.sqlite3")).unwrap();
        let mut db = connect(library.database_path()).unwrap();
        for (id, name) in [(1, "one.flac"), (2, "two.flac")] {
            let file = root.join(name);
            fs::write(&file, b"old audio").unwrap();
            db.execute(
                "INSERT INTO track (id,path,stamp,size,metadata) VALUES (?1,?2,'',9,'{}')",
                params![id, file.to_str().unwrap()],
            )
            .unwrap();
            db.execute("INSERT INTO source VALUES ('library',?1)", [id])
                .unwrap();
            stage_track(&mut db, &file).unwrap();
        }
        let staged = |id| -> String {
            db.query_row(
                "SELECT directory FROM track_trash WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
        };
        let first = PathBuf::from(staged(1)).join("audio");
        let second = PathBuf::from(staged(2)).join("audio");
        let replacement = root.join("one.flac");
        fs::write(&replacement, b"new audio").unwrap();

        assert_eq!(purge(&db, Some(1), None).unwrap(), 0);
        assert!(!first.exists());
        assert!(second.exists());
        assert_eq!(fs::read(&replacement).unwrap(), b"new audio");
        assert_eq!(purge(&db, None, None).unwrap(), 0);
        assert!(!second.exists());
        let count: u32 = db
            .query_row("SELECT COUNT(*) FROM track_trash", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn restoring_preserves_track_identity_and_library_membership() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().canonicalize().unwrap().join("track.flac");
        fs::write(&original, b"audio").unwrap();
        let library = Library::new(temp.path().join("library.sqlite3")).unwrap();
        let mut db = connect(library.database_path()).unwrap();
        db.execute(
            "INSERT INTO track (id,path,stamp,size,metadata,play_count) VALUES (1,?1,'',5,?2,7)",
            params![
                original.to_str().unwrap(),
                serde_json::to_string(&super::super::Tags::default()).unwrap()
            ],
        )
        .unwrap();
        db.execute("INSERT INTO source VALUES ('library',1)", [])
            .unwrap();
        stage_track(&mut db, &original).unwrap();
        restore_entry(&mut db, 1, &["library".into()]).unwrap();
        let plays: i64 = db.query_row("SELECT play_count FROM track JOIN source ON source.track_id=track.id WHERE track.id=1", [], |row| row.get(0)).unwrap();
        assert_eq!(plays, 7);
        assert_eq!(fs::read(&original).unwrap(), b"audio");
    }

    #[test]
    fn legacy_entry_and_replacement_at_same_path_can_be_restored_independently() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("library.sqlite3");
        drop(Library::new(database.clone()).unwrap());
        let root = temp.path().canonicalize().unwrap();
        let roots = [root.to_str().unwrap().to_owned()];
        let original = root.join("replacement.flac");
        let legacy = root.join(".yap-trash-legacy");
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join("audio"), b"original audio").unwrap();
        let db = connect(&database).unwrap();
        db.execute_batch(
            "DROP TABLE track_trash;
             CREATE TABLE track_trash (track_id INTEGER PRIMARY KEY REFERENCES track(id),
                removed_at INTEGER NOT NULL, directory TEXT NOT NULL, roots TEXT NOT NULL);
             DROP TABLE release_artwork; DROP TABLE resolver_artwork;
             PRAGMA user_version=13;",
        )
        .unwrap();
        let tags = super::super::Tags {
            name: "Original".into(),
            ..Default::default()
        };
        db.execute(
            "INSERT INTO track (id,path,stamp,size,metadata) VALUES (42,?1,'',14,?2)",
            params![original.to_str(), serde_json::to_string(&tags).unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO track_trash VALUES (42,unixepoch(),?1,?2)",
            params![legacy.to_str(), serde_json::to_string(&roots).unwrap()],
        )
        .unwrap();
        drop(db);
        let library = Library::new(database).unwrap();
        let mut db = connect(library.database_path()).unwrap();

        fs::write(&original, b"replacement audio").unwrap();
        super::super::scan(
            &mut db,
            &roots,
            &Default::default(),
            &mut Default::default(),
            |_| {},
        )
        .unwrap();
        assert!(restore_entry(&mut db, 42, &roots).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"replacement audio");
        stage_track(&mut db, &original).unwrap();
        let replacement_id: i64 = db
            .query_row(
                "SELECT id FROM track_trash WHERE track_id=42 AND id<>42",
                [],
                |row| row.get(0),
            )
            .unwrap();

        restore_entry(&mut db, 42, &roots).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"original audio");
        let title: String = db
            .query_row(
                "SELECT json_extract(metadata,'$.name') FROM track WHERE id=42",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Original");
        stage_track(&mut db, &original).unwrap();
        db.execute(
            "UPDATE track_trash SET removed_at=unixepoch()-31*86400 WHERE id<>?1",
            [replacement_id],
        )
        .unwrap();
        remove_expired(&db).unwrap();
        restore_entry(&mut db, replacement_id, &roots).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"replacement audio");
    }
}
