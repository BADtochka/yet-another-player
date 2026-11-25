use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    audio::{self, Action, AudioEngine, PairAnalysis, PairSettings, PlaylistMix, Snapshot},
    MusicFolders,
};

use super::{connect, queries, Library, LibraryResult, Track};

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

pub(super) fn migrate_transitions(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS playlist_mix (
            playlist_id INTEGER PRIMARY KEY REFERENCES playlist(id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS playlist_transition (
            playlist_id INTEGER NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
            outgoing_id INTEGER NOT NULL REFERENCES track(id) ON DELETE CASCADE,
            incoming_id INTEGER NOT NULL REFERENCES track(id) ON DELETE CASCADE,
            settings TEXT NOT NULL,
            PRIMARY KEY (playlist_id, outgoing_id, incoming_id)
         );
         PRAGMA user_version=17;",
    )?;
    tx.commit()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    id: i64,
    name: String,
    track_ids: Vec<i64>,
    mix_enabled: bool,
    transitions: Vec<SavedTransition>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedTransition {
    outgoing_id: i64,
    incoming_id: i64,
    settings: PairSettings,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Change {
    List,
    Create {
        name: String,
        #[serde(default)]
        mix_enabled: bool,
    },
    Rename {
        id: i64,
        name: String,
    },
    Delete {
        id: i64,
    },
    Add {
        id: i64,
        track_id: i64,
    },
    Remove {
        id: i64,
        track_id: i64,
    },
    Move {
        id: i64,
        track_id: i64,
        later: bool,
    },
    SetMix {
        id: i64,
        enabled: bool,
    },
    SetTransition {
        id: i64,
        outgoing_id: i64,
        incoming_id: i64,
        settings: Option<PairSettings>,
    },
}

fn read_transitions(db: &Connection, id: i64) -> LibraryResult<Vec<SavedTransition>> {
    let mut statement = db.prepare(
        "SELECT outgoing_id, incoming_id, settings FROM playlist_transition WHERE playlist_id=?1",
    )?;
    let mut rows = statement.query([id])?;
    let mut saved = Vec::new();
    while let Some(row) = rows.next()? {
        let json: String = row.get(2)?;
        saved.push(SavedTransition {
            outgoing_id: row.get(0)?,
            incoming_id: row.get(1)?,
            settings: serde_json::from_str::<PairSettings>(&json)?.validated(),
        });
    }
    Ok(saved)
}

fn playlist_tracks(db: &Connection, id: i64) -> LibraryResult<Vec<Track>> {
    queries::read_collection_tracks(db, queries::CollectionKind::Playlist, &id.to_string())
}

fn adjacent(tracks: &[Track], outgoing: i64, incoming: i64) -> LibraryResult<(&Track, &Track)> {
    let pair = tracks
        .windows(2)
        .find(|pair| pair[0].id == outgoing && pair[1].id == incoming)
        .ok_or("Треки больше не стоят рядом в плейлисте")?;
    Ok((&pair[0], &pair[1]))
}

fn pair_paths(
    db: &Connection,
    roots: &Mutex<Vec<PathBuf>>,
    id: i64,
    outgoing_id: i64,
    incoming_id: i64,
) -> LibraryResult<(PathBuf, PathBuf)> {
    let tracks = playlist_tracks(db, id)?;
    let (outgoing, incoming) = adjacent(&tracks, outgoing_id, incoming_id)?;
    Ok((
        audio::library_track_path(outgoing.path(), roots)?,
        audio::library_track_path(incoming.path(), roots)?,
    ))
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

pub(super) fn change_playlists(db: &mut Connection, change: Change) -> LibraryResult<Vec<Playlist>> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    match change {
        Change::List => {}
        Change::Create { name, mix_enabled } => {
            tx.execute(
                "INSERT INTO playlist (name) VALUES (?1)",
                [name_for_write(&name)?],
            )?;
            if mix_enabled {
                tx.execute(
                    "INSERT INTO playlist_mix VALUES (?1)",
                    [tx.last_insert_rowid()],
                )?;
            }
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
                "DELETE FROM playlist_track WHERE playlist_id = ?1 AND
                 (track_id = ?2 OR track_id IN (SELECT variant_id FROM track_variant WHERE audio_id=?2))",
                params![id, track_id],
            )?;
            tx.execute(
                "DELETE FROM playlist_transition WHERE playlist_id=?1 AND (outgoing_id=?2 OR incoming_id=?2)",
                params![id, track_id],
            )?;
        }
        Change::Move {
            id,
            track_id,
            later,
        } => {
            let track_id = tx.query_row(
                "SELECT track_id FROM playlist_track WHERE playlist_id=?1 AND
                 (track_id=?2 OR track_id IN (SELECT variant_id FROM track_variant WHERE audio_id=?2))
                 ORDER BY position LIMIT 1",
                params![id, track_id], |row| row.get(0),
            ).optional()?.unwrap_or(track_id);
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
        Change::SetMix { id, enabled } => {
            if enabled {
                tx.execute("INSERT OR IGNORE INTO playlist_mix VALUES (?1)", [id])?;
            } else {
                tx.execute("DELETE FROM playlist_mix WHERE playlist_id=?1", [id])?;
            }
        }
        Change::SetTransition {
            id,
            outgoing_id,
            incoming_id,
            settings,
        } => {
            if let Some(settings) = settings {
                adjacent(&playlist_tracks(&tx, id)?, outgoing_id, incoming_id)?;
                let json = serde_json::to_string(&settings.validated())?;
                tx.execute(
                    "INSERT INTO playlist_transition (playlist_id, outgoing_id, incoming_id, settings)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (playlist_id, outgoing_id, incoming_id) DO UPDATE SET settings=excluded.settings",
                    params![id, outgoing_id, incoming_id, json],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM playlist_transition WHERE playlist_id=?1 AND outgoing_id=?2 AND incoming_id=?3",
                    params![id, outgoing_id, incoming_id],
                )?;
            }
        }
    }
    let playlists = {
        let mut stmt = tx.prepare(
            "SELECT id, name, EXISTS (SELECT 1 FROM playlist_mix WHERE playlist_id=playlist.id)
             FROM playlist ORDER BY id",
        )?;
        let rows = stmt.query_map((), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        let mut playlists = Vec::new();
        for row in rows {
            let (id, name, mix_enabled) = row?;
            playlists.push(Playlist {
                id,
                name,
                track_ids: read_track_ids(&tx, id)?,
                mix_enabled,
                transitions: read_transitions(&tx, id)?,
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

#[tauri::command]
pub async fn library_playlist_transition_analysis(
    state: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    id: i64,
    outgoing_id: i64,
    incoming_id: i64,
) -> Result<PairAnalysis, String> {
    let db_path = state.database.clone();
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<_> {
        let (from, to) = pair_paths(&connect(&db_path)?, &roots, id, outgoing_id, incoming_id)?;
        Ok(audio::analyze_pair(&from, &to, &roots, &db_path))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn library_preview_playlist_transition(
    state: State<'_, Library>,
    audio: State<'_, AudioEngine>,
    folders: State<'_, MusicFolders>,
    preview_id: u64,
    id: i64,
    outgoing_id: i64,
    incoming_id: i64,
    settings: PairSettings,
) -> Result<Snapshot, String> {
    let db_path = state.database.clone();
    let roots = Arc::clone(&folders.canonical);
    let (outgoing, incoming) = tauri::async_runtime::spawn_blocking(move || {
        pair_paths(&connect(&db_path)?, &roots, id, outgoing_id, incoming_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    audio
        .control(Action::StartTransitionPreview {
            id: preview_id,
            outgoing,
            incoming,
            settings,
        })
        .await
}

#[tauri::command]
pub async fn library_play_playlist(
    state: State<'_, Library>,
    audio: State<'_, AudioEngine>,
    id: i64,
    track_id: i64,
) -> Result<Snapshot, String> {
    let request = audio.reserve_queue();
    let path = state.database.clone();
    let action = tauri::async_runtime::spawn_blocking(move || {
        playlist_queue(&mut connect(&path)?, id, track_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    audio.control_reserved_queue(request, action).await
}

fn playlist_queue(db: &mut Connection, id: i64, track_id: i64) -> LibraryResult<Action> {
    let tx = db.transaction()?;
    let enabled: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM playlist_mix WHERE playlist_id=?1) FROM playlist WHERE id=?1",
        [id],
        |row| row.get(0),
    )?;
    let tracks = playlist_tracks(&tx, id)?;
    let index = tracks
        .iter()
        .position(|track| track.id == track_id)
        .ok_or("Трек отсутствует в плейлисте")?;
    let playlist_mix = if enabled {
        let saved = read_transitions(&tx, id)?
            .into_iter()
            .map(|item| ((item.outgoing_id, item.incoming_id), item.settings))
            .collect::<std::collections::HashMap<_, _>>();
        Some(
            tracks
                .windows(2)
                .filter_map(|pair| {
                    let settings = saved.get(&(pair[0].id, pair[1].id))?;
                    Some((pair[0].path.clone(), (pair[1].path.clone(), *settings)))
                })
                .collect::<PlaylistMix>(),
        )
    } else {
        None
    };
    Ok(Action::Queue {
        tracks: tracks
            .into_iter()
            .map(|track| audio::Track::local(track.path, track.tags.replay_gain_db))
            .collect(),
        index,
        position: 0.0,
        playing: true,
        restart_shuffle: false,
        playlist_mix,
    })
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
        let playlists = change_playlists(
            &mut db,
            Change::Create {
                name: "Mix".into(),
                mix_enabled: false,
            },
        )
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

    #[test]
    fn saved_transitions_survive_reopen_and_only_apply_to_adjacent_tracks() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("library.sqlite3");
        Library::new(path.clone()).unwrap();
        let mut db = connect(&path).unwrap();
        for name in ["first", "second", "third"] {
            db.execute(
                "INSERT INTO track (path, stamp, size, metadata) VALUES (?1, '', 0, ?2)",
                params![
                    name,
                    serde_json::json!({ "name": name, "artists": [], "lyrics": [] }).to_string()
                ],
            )
            .unwrap();
        }
        db.execute("INSERT INTO source SELECT 'fixture', id FROM track", [])
            .unwrap();
        let id = change_playlists(
            &mut db,
            Change::Create {
                name: "Mix".into(),
                mix_enabled: true,
            },
        )
        .unwrap()[0]
            .id;
        for track_id in 1..=3 {
            change_playlists(&mut db, Change::Add { id, track_id }).unwrap();
        }
        let settings: PairSettings = serde_json::from_str(
            r#"{"preset":"bassSwap","seconds":3.4,"trim":0.5,"cue":0.2,"intensity":0.6}"#,
        )
        .unwrap();
        change_playlists(
            &mut db,
            Change::SetTransition {
                id,
                outgoing_id: 1,
                incoming_id: 2,
                settings: Some(settings),
            },
        )
        .unwrap();
        drop(db);

        let mut db = connect(&path).unwrap();
        let Action::Queue {
            playlist_mix: Some(mix),
            ..
        } = playlist_queue(&mut db, id, 1).unwrap()
        else {
            panic!("The mixed playlist should load its saved plan")
        };
        assert_eq!(mix.get("first"), Some(&("second".into(), settings)));
        change_playlists(
            &mut db,
            Change::Move {
                id,
                track_id: 2,
                later: true,
            },
        )
        .unwrap();
        let Action::Queue {
            playlist_mix: Some(mix),
            ..
        } = playlist_queue(&mut db, id, 1).unwrap()
        else {
            panic!("The playlist should remain mixed")
        };
        assert!(mix.is_empty());
        assert!(change_playlists(
            &mut db,
            Change::SetTransition {
                id,
                outgoing_id: 1,
                incoming_id: 2,
                settings: Some(settings)
            }
        )
        .is_err());
    }
}
