use std::path::Path;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use tauri::State;

use super::super::{connect, user_error, Library, LibraryResult};

const MIB: i64 = 1024 * 1024;
const DEFAULT_MIB: i64 = 512;
const SAVED_RECORD: &str = r#"
    query LIKE '["artist-profile",%' OR query LIKE '["artist-avatar",%'
    OR query LIKE '["release-details",%' OR query LIKE '["track-release",%'
    OR query LIKE '["album-artwork",%'
"#;

pub(in crate::library) fn migrate(db: &mut Connection) -> LibraryResult<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS catalog_storage (
        id INTEGER PRIMARY KEY CHECK (id=1), max_bytes INTEGER NOT NULL
    ); PRAGMA user_version=18;",
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO catalog_storage VALUES (1, ?1)",
        [DEFAULT_MIB * MIB],
    )?;
    trim(&tx)?;
    tx.commit()?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageStatus {
    max_mib: i64,
    used_bytes: i64,
}

fn status(db: &Connection) -> LibraryResult<StorageStatus> {
    Ok(db.query_row(
        &format!(
            "SELECT max_bytes / {MIB},
            (SELECT COALESCE(SUM(length(bytes)),0) FROM resolver_artwork)
            + (SELECT COALESCE(SUM(length(CAST(query AS BLOB)) + length(CAST(response AS BLOB))),0)
               FROM resolver_cache WHERE {SAVED_RECORD})
            FROM catalog_storage WHERE id=1"
        ),
        [],
        |row| {
            Ok(StorageStatus {
                max_mib: row.get(0)?,
                used_bytes: row.get(1)?,
            })
        },
    )?)
}

fn trim(db: &Connection) -> LibraryResult<bool> {
    let status = status(db)?;
    let over = status.used_bytes - status.max_mib * MIB;
    if over <= 0 {
        return Ok(false);
    }
    // Keep metadata longer than image bytes. Each group is removed in save order.
    db.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS evicted_catalog (
        is_image INTEGER NOT NULL, key TEXT NOT NULL
    ); DELETE FROM evicted_catalog;",
    )?;
    db.execute(
        &format!("INSERT INTO evicted_catalog
         SELECT is_image, key FROM (
            SELECT is_image, key, SUM(bytes) OVER (ORDER BY is_image DESC, seq) - bytes AS prior_bytes
            FROM (
                SELECT 1 AS is_image, url AS key, rowid AS seq, length(bytes) AS bytes FROM resolver_artwork
                UNION ALL
                SELECT 0, query, rowid, length(CAST(query AS BLOB)) + length(CAST(response AS BLOB))
                FROM resolver_cache WHERE {SAVED_RECORD}
            )
         ) WHERE prior_bytes < ?1"),
        [over],
    )?;
    db.execute_batch(
        "DELETE FROM release_artwork WHERE url IN
        (SELECT key FROM evicted_catalog WHERE is_image=1);
        DELETE FROM resolver_artwork WHERE url IN
        (SELECT key FROM evicted_catalog WHERE is_image=1);
        DELETE FROM resolver_cache WHERE query IN
        (SELECT key FROM evicted_catalog WHERE is_image=0);",
    )?;
    Ok(true)
}

pub(super) fn save_record(database: &Path, key: &str, response: &str) -> LibraryResult<()> {
    let mut db = connect(database)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT OR REPLACE INTO resolver_cache VALUES (?1,unixepoch(),?2)",
        params![key, response],
    )?;
    trim(&tx)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn save_artwork(database: &Path, url: &str, bytes: &[u8]) -> LibraryResult<bool> {
    let mut db = connect(database)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT OR IGNORE INTO resolver_artwork (url, bytes) VALUES (?1, ?2)",
        params![url, bytes],
    )?;
    trim(&tx)?;
    let retained = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM resolver_artwork WHERE url=?1)",
        [url],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(retained)
}

#[tauri::command]
pub(crate) async fn library_catalog_storage(
    state: State<'_, Library>,
    max_mib: Option<i64>,
) -> Result<StorageStatus, String> {
    if max_mib.is_some_and(|value| !(16..=16_384).contains(&value)) {
        return Err("Лимит должен быть от 16 до 16 384 МБ.".into());
    }
    let database = state.database_path().to_owned();
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<_> {
        let mut db = connect(&database)?;
        if let Some(limit) = max_mib {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE catalog_storage SET max_bytes=?1 WHERE id=1",
                [limit * MIB],
            )?;
            let removed = trim(&tx)?;
            tx.commit()?;
            if removed {
                // The main database retains free pages until it is compacted.
                if let Err(error) = db.execute_batch("VACUUM") {
                    eprintln!("Could not compact catalog storage: {error}");
                }
            }
        }
        status(&db)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(user_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_evicts_old_artwork_and_its_links_without_removing_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        let library = Library::new(path.clone()).unwrap();
        let db = connect(&path).unwrap();
        db.execute("UPDATE catalog_storage SET max_bytes=?1", [MIB])
            .unwrap();
        let bytes = vec![0; MIB as usize / 2];
        assert!(save_artwork(&path, "old", &bytes).unwrap());
        db.execute("INSERT INTO release_artwork VALUES ('album', 'old')", [])
            .unwrap();
        db.execute(
            "INSERT INTO track(path, stamp, size, metadata) VALUES ('song', '', 0, '{}')",
            [],
        )
        .unwrap();
        save_record(
            &path,
            r#"["artist-profile",4,"artist"]"#,
            r#"{"releases":[]}"#,
        )
        .unwrap();
        assert!(save_artwork(&path, "new", &bytes).unwrap());
        drop(db);
        drop(library);
        let reopened = Library::new(path.clone()).unwrap();
        let db = connect(reopened.database_path()).unwrap();
        let counts = db
            .query_row(
                "SELECT (SELECT count(*) FROM resolver_cache),
            (SELECT count(*) FROM resolver_artwork WHERE url='new'),
            (SELECT count(*) FROM release_artwork), (SELECT count(*) FROM track)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(counts, (1, 1, 0, 1));
        assert!(status(&db).unwrap().used_bytes <= MIB);
    }
}
