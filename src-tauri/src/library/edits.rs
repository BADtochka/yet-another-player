use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use super::{connect, queries, user_error, versions, Library, LibraryResult, Tags};

const MAX_OFFSET: i32 = 60_000;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Fields {
    name: String,
    artists: Vec<String>,
    album: Option<String>,
    album_artist: Option<String>,
    release_date: Option<String>,
    genres: Vec<String>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Field {
    Name,
    Artists,
    Album,
    AlbumArtist,
    ReleaseDate,
    Genres,
    TrackNumber,
    DiscNumber,
}

fn clean(value: &mut String) -> Result<(), String> {
    *value = value.trim().to_owned();
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err("Слишком длинное или недопустимое значение.".into());
    }
    Ok(())
}

impl Fields {
    fn validate(&mut self) -> Result<(), String> {
        clean(&mut self.name)?;
        if self.name.is_empty() || self.artists.len() > 32 || self.genres.len() > 32 {
            return Err("Укажите название трека и проверьте списки исполнителей и жанров.".into());
        }
        for value in self.artists.iter_mut().chain(self.genres.iter_mut()) {
            clean(value)?;
        }
        self.artists.retain(|value| !value.is_empty());
        self.genres.retain(|value| !value.is_empty());
        for field in [
            &mut self.album,
            &mut self.album_artist,
            &mut self.release_date,
        ] {
            if let Some(value) = field {
                clean(value)?;
                if value.is_empty() {
                    *field = None;
                }
            }
        }
        if let Some(date) = &self.release_date {
            let parts: Vec<_> = date.split('-').collect();
            let valid_year =
                parts[0].len() == 4 && parts[0].parse::<u16>().is_ok_and(|year| year > 0);
            let valid = valid_year
                && match parts.as_slice() {
                    [_] => true,
                    [_, month] => {
                        month.len() == 2
                            && month
                                .parse::<u8>()
                                .is_ok_and(|month| (1..=12).contains(&month))
                    }
                    [_, month, day] => {
                        month.len() == 2
                            && day.len() == 2
                            && date
                                .parse::<lofty::tag::items::Timestamp>()
                                .ok()
                                .is_some_and(|parsed| {
                                    let (Some(month), Some(day)) = (parsed.month, parsed.day)
                                    else {
                                        return false;
                                    };
                                    let year = parsed.year;
                                    let leap =
                                        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                                    day > 0
                                        && day
                                            <= match month {
                                                2 => 28 + u8::from(leap),
                                                4 | 6 | 9 | 11 => 30,
                                                1..=12 => 31,
                                                _ => 0,
                                            }
                                })
                    }
                    _ => false,
                };
            if !valid {
                return Err("Укажите реальную дату в формате ГГГГ, ГГГГ-ММ или ГГГГ-ММ-ДД.".into());
            }
        }
        Ok(())
    }
}

pub(super) fn apply(db: &Connection, path: &str, tags: &mut Tags) -> LibraryResult<()> {
    let edit: Option<(String, i32)> = db.query_row(
        "SELECT e.fields, e.offset_ms FROM track_edits e JOIN track t ON t.id=e.track_id WHERE t.path=?1",
        [path], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let (mut fields, offset) = match edit {
        Some((fields, offset)) => (serde_json::from_str::<Map<String, Value>>(&fields)?, Some(offset)),
        None => (Map::new(), None),
    };
    super::sync::extend_metadata_edits(db, path, &mut fields)?;
    if !fields.is_empty() || offset.is_some() {
        let mut original = serde_json::to_value(&*tags)?;
        let values = original.as_object_mut().ok_or("Invalid track metadata")?;
        if let Some(date) = fields.get("releaseDate") {
            let year = date
                .as_str()
                .and_then(|value| value.get(..4))
                .and_then(|year| year.parse::<u16>().ok());
            values.insert("year".into(), json!(year));
        }
        if let Some(genres) = fields.get("genres") {
            let genre = genres
                .as_array()
                .and_then(|genres| genres.first())
                .cloned()
                .unwrap_or(Value::Null);
            values.insert("genre".into(), genre);
        }
        if let Some(album) = fields.get("album") {
            if values.get("album") != Some(album) {
                values.remove("isSingle");
                values.remove("isEp");
            }
        }
        if fields.contains_key("name") {
            values.insert("fallbackTitle".into(), json!(false));
        }
        values.append(&mut fields);
        if let Some(offset) = offset {
            values.insert("lyricsOffsetMs".into(), json!(offset));
        }
        *tags = serde_json::from_value(original)?;
    }
    Ok(())
}

struct LyricsOffsets {
    track: Option<i32>,
    clip: Option<i32>,
}

fn write_edit(
    db: &Connection,
    id: i64,
    tags: &Tags,
    fields: Option<Map<String, Value>>,
    offsets: LyricsOffsets,
) -> LibraryResult<()> {
    let previous: Option<(String, i32)> = db
        .query_row(
            "SELECT fields, offset_ms FROM track_edits WHERE track_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (mut saved, track_offset) = match previous {
        Some((fields, offset)) => (serde_json::from_str::<Map<String, Value>>(&fields)?, offset),
        None => (Map::new(), tags.lyrics_offset_ms),
    };
    if let Some(fields) = fields {
        saved.extend(fields);
    }
    if let Some(offset) = offsets.clip {
        saved.insert("clipLyricsOffsetMs".into(), json!(offset));
    } else if offsets.track.is_some_and(|offset| offset != track_offset) {
        // Legacy tracks used the same offset in both modes. Keep the old clip timing on the first audio edit.
        saved
            .entry("clipLyricsOffsetMs")
            .or_insert_with(|| json!(tags.clip_lyrics_offset_ms.unwrap_or(track_offset)));
    }
    db.execute(
        "INSERT INTO track_edits (track_id, fields, offset_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(track_id) DO UPDATE SET fields=excluded.fields, offset_ms=excluded.offset_ms",
        params![
            id,
            serde_json::to_string(&saved)?,
            offsets.track.unwrap_or(track_offset)
        ],
    )?;
    Ok(())
}

async fn save(
    app: &AppHandle,
    library: &Library,
    id: i64,
    fields: Option<(Fields, Vec<Field>)>,
    offsets: LyricsOffsets,
) -> Result<(), String> {
    if [offsets.track, offsets.clip]
        .into_iter()
        .flatten()
        .any(|offset| !(-MAX_OFFSET..=MAX_OFFSET).contains(&offset))
    {
        return Err("Смещение должно быть от −60 000 до 60 000 мс.".into());
    }
    let fields = fields
        .map(|(mut fields, changed)| {
            fields.validate()?;
            let values = serde_json::to_value(fields).map_err(|err| err.to_string())?;
            let selected = serde_json::to_value(changed).map_err(|err| err.to_string())?;
            let Value::Object(mut values) = values else {
                return Err("Некорректные метаданные".to_owned());
            };
            values.retain(|name, _| {
                selected
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some(name)))
            });
            Ok(values)
        })
        .transpose()?
        .filter(|values| !values.is_empty());
    if fields.is_none() && offsets.track.is_none() && offsets.clip.is_none() {
        return Ok(());
    }
    let writer = library.lock_catalog_writer().await;
    let database = library.database_path().to_owned();
    let lyrics_changed = offsets.track.is_some();
    let path = tauri::async_runtime::spawn_blocking(move || -> LibraryResult<String> {
        let _writer = writer;
        let mut db = connect(&database)?;
        let tx = db.transaction()?;
        let (path, metadata): (String, String) = tx.query_row(
            "SELECT path, metadata FROM track WHERE id=?1 AND EXISTS (SELECT 1 FROM source WHERE track_id=?1)",
            [id], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let fields_changed = fields.is_some();
        let mut tags = serde_json::from_str(&metadata)?;
        if let Some(fields) = &fields {
            super::sync::queue_metadata_edits(&tx, id, fields)?;
        }
        write_edit(&tx, id, &tags, fields, offsets)?;
        apply(&tx, &path, &mut tags)?;
        tx.execute("UPDATE track SET metadata=?2 WHERE id=?1", params![id, serde_json::to_string(&tags)?])?;
        if fields_changed {
            queries::index(&tx, id, &path, &tags)?;
            versions::rebuild(&tx)?;
        }
        tx.execute("UPDATE catalog_state SET revision=revision+1", [])?;
        tx.commit()?;
        Ok(path)
    }).await.map_err(|err| err.to_string())?.map_err(user_error)?;
    if lyrics_changed {
        app.state::<crate::audio::AudioEngine>()
            .lyrics_changed(&path);
    }
    let _ = app.emit("library:metadata", ());
    Ok(())
}

#[tauri::command]
pub(crate) async fn library_edit_track(
    app: AppHandle,
    state: State<'_, Library>,
    id: i64,
    fields: Fields,
    changed: Vec<Field>,
    offset_ms: Option<i32>,
    clip_offset_ms: Option<i32>,
) -> Result<(), String> {
    save(
        &app,
        &state,
        id,
        Some((fields, changed)),
        LyricsOffsets {
            track: offset_ms,
            clip: clip_offset_ms,
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn library_set_lyrics_offset(
    app: AppHandle,
    state: State<'_, Library>,
    id: i64,
    offset_ms: Option<i32>,
    clip_offset_ms: Option<i32>,
) -> Result<(), String> {
    save(
        &app,
        &state,
        id,
        None,
        LyricsOffsets {
            track: offset_ms,
            clip: clip_offset_ms,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reapplies_only_edited_fields_and_the_offset_after_reading_tags() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::new(dir.path().join("catalog.sqlite3")).unwrap();
        let db = connect(library.database_path()).unwrap();
        db.execute(
            "INSERT INTO track (id,path,stamp,size,metadata) VALUES (1,'path','',0,'{}')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO track_edits VALUES (1,?1,125)",
            [r#"{"name":"Edited","album":null}"#],
        )
        .unwrap();
        let mut tags = Tags {
            name: "Original".into(),
            album: Some("Original album".into()),
            genres: vec!["Rock".into()],
            ..Tags::default()
        };
        apply(&db, "path", &mut tags).unwrap();
        assert_eq!(tags.name, "Edited");
        assert_eq!(tags.album, None);
        assert_eq!(tags.genres, ["Rock"]);
        assert_eq!(tags.lyrics_offset_ms, 125);
    }

    #[test]
    fn keeps_track_and_clip_offsets_independent_after_legacy_edit() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::new(dir.path().join("catalog.sqlite3")).unwrap();
        let db = connect(library.database_path()).unwrap();
        db.execute(
            "INSERT INTO track (id,path,stamp,size,metadata) VALUES (1,'path','',0,'{}')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO track_edits VALUES (1,?1,125)",
            [r#"{"name":"Edited"}"#],
        )
        .unwrap();

        let mut tags = Tags::default();
        apply(&db, "path", &mut tags).unwrap();
        assert_eq!(tags.clip_lyrics_offset_ms, None);

        for (track, clip, expected_track, expected_clip) in [
            (Some(400), None, 400, 125),
            (None, Some(0), 400, 0),
            (Some(-200), None, -200, 0),
        ] {
            write_edit(&db, 1, &tags, None, LyricsOffsets { track, clip }).unwrap();
            // A rescan starts from fresh file tags.
            tags = Tags::default();
            apply(&db, "path", &mut tags).unwrap();
            assert_eq!(tags.lyrics_offset_ms, expected_track);
            assert_eq!(tags.clip_lyrics_offset_ms, Some(expected_clip));
            assert_eq!(tags.name, "Edited");
        }
    }
}
