use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{connect, Library, LibraryResult};

pub(super) fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE playlist (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL
        );
        CREATE TABLE playlist_track (
            playlist_id INTEGER NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
            track_id INTEGER NOT NULL REFERENCES track(id),
            position INTEGER NOT NULL,
            PRIMARY KEY (playlist_id, track_id)
        );
        CREATE INDEX playlist_order ON playlist_track (playlist_id, position);
        PRAGMA user_version = 2;",
    )?;
    tx.commit()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    id: i64,
    name: String,
    track_ids: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Change {
    List,
    Create { name: String },
    Rename { id: i64, name: String },
    Delete { id: i64 },
    Add { id: i64, track_id: i64 },
    Remove { id: i64, track_id: i64 },
    Move { id: i64, track_id: i64, later: bool },
}

fn read_track_ids(db: &Connection, id: i64) -> rusqlite::Result<Vec<i64>> {
    let mut stmt =
        db.prepare("SELECT track_id FROM playlist_track WHERE playlist_id = ?1 ORDER BY position")?;
    let ids = stmt.query_map([id], |row| row.get(0))?;
    ids.collect()
}

fn name_for_write(name: &str) -> LibraryResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err("Название должно содержать от 1 до 120 символов".into());
    }
    Ok(name)
}

fn change_playlists(db: &mut Connection, change: Change) -> LibraryResult<Vec<Playlist>> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    match change {
        Change::List => {}
        Change::Create { name } => {
            tx.execute(
                "INSERT INTO playlist (name) VALUES (?1)",
                [name_for_write(&name)?],
            )?;
        }
        Change::Rename { id, name } => {
            tx.execute(
                "UPDATE playlist SET name = ?1 WHERE id = ?2",
                params![name_for_write(&name)?, id],
            )?;
        }
        Change::Delete { id } => {
            tx.execute("DELETE FROM playlist WHERE id = ?1", [id])?;
        }
        Change::Add { id, track_id } => {
            tx.execute(
                "INSERT OR IGNORE INTO playlist_track
                 SELECT ?1, ?2, COALESCE(MAX(position) + 1, 0)
                 FROM playlist_track WHERE playlist_id = ?1",
                params![id, track_id],
            )?;
        }
        Change::Remove { id, track_id } => {
            tx.execute(
                "DELETE FROM playlist_track WHERE playlist_id = ?1 AND track_id = ?2",
                params![id, track_id],
            )?;
        }
        Change::Move {
            id,
            track_id,
            later,
        } => {
            let ids = read_track_ids(&tx, id)?;
            if let Some(index) = ids.iter().position(|current| *current == track_id) {
                let neighbor = if later {
                    index.checked_add(1)
                } else {
                    index.checked_sub(1)
                };
                if let Some(other) = neighbor.and_then(|index| ids.get(index)) {
                    let first_position: i64 = tx.query_row(
                        "SELECT position FROM playlist_track WHERE playlist_id = ?1 AND track_id = ?2",
                        params![id, track_id],
                        |row| row.get(0),
                    )?;
                    let second_position: i64 = tx.query_row(
                        "SELECT position FROM playlist_track WHERE playlist_id = ?1 AND track_id = ?2",
                        params![id, other],
                        |row| row.get(0),
                    )?;
                    tx.execute(
                        "UPDATE playlist_track SET position = CASE track_id WHEN ?2 THEN ?4 ELSE ?5 END
                         WHERE playlist_id = ?1 AND track_id IN (?2, ?3)",
                        params![id, track_id, other, second_position, first_position],
                    )?;
                }
            }
        }
    }
    let playlists = {
        let mut stmt = tx.prepare("SELECT id, name FROM playlist ORDER BY id")?;
        let rows = stmt.query_map((), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut playlists = Vec::new();
        for row in rows {
            let (id, name) = row?;
            playlists.push(Playlist {
                id,
                name,
                track_ids: read_track_ids(&tx, id)?,
            });
        }
        playlists
    };
    tx.commit()?;
    Ok(playlists)
}

#[tauri::command]
pub async fn library_playlists(
    state: State<'_, Library>,
    change: Change,
) -> Result<Vec<Playlist>, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || change_playlists(&mut connect(&path)?, change))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_order_survives_reopen_and_duplicate_add() {
        let dir = tempfile::tempdir().expect("test directory");
        let path = dir.path().join("library.sqlite3");
        Library::new(path.clone()).expect("initialize catalog");
        let mut db = connect(&path).expect("open catalog");
        for name in ["first", "second", "third"] {
            db.execute(
                "INSERT INTO track (path, stamp, size, metadata) VALUES (?1, '', 0, '{}')",
                [name],
            )
            .expect("insert fixture track");
        }
        let playlists = change_playlists(&mut db, Change::Create { name: "Mix".into() })
            .expect("create playlist");
        let id = playlists[0].id;
        for track_id in [1, 2, 3, 2] {
            change_playlists(&mut db, Change::Add { id, track_id }).expect("add track");
        }
        change_playlists(
            &mut db,
            Change::Move {
                id,
                track_id: 3,
                later: false,
            },
        )
        .expect("move track");
        drop(db);

        let mut db = connect(&path).expect("reopen catalog");
        let playlists = change_playlists(&mut db, Change::List).expect("read playlists");
        assert_eq!(playlists[0].track_ids, [1, 3, 2]);
    }
}
