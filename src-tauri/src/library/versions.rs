use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use super::{LibraryResult, Tags};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(super) fn migrate(db: &mut Connection) -> LibraryResult<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE track_variant (
            variant_id INTEGER PRIMARY KEY REFERENCES track(id),
            audio_id INTEGER NOT NULL REFERENCES track(id),
            local_video INTEGER NOT NULL,
            clip_id TEXT
         );
         CREATE INDEX track_variant_audio ON track_variant(audio_id);
         PRAGMA user_version=8;",
    )?;
    rebuild(&tx)?;
    tx.execute("UPDATE catalog_state SET revision=revision+1", [])?;
    tx.commit()?;
    Ok(())
}

pub(super) fn refresh_local_video(db: &mut Connection) -> LibraryResult<()> {
    let tx = db.transaction()?;
    let paths = {
        let mut statement =
            tx.prepare("SELECT id, path FROM track WHERE lower(path) LIKE '%.mp4'")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, path) in paths {
        if has_video(Path::new(&path)) {
            tx.execute("UPDATE track SET metadata=json_set(metadata,'$.hasLocalVideo',json('true')) WHERE id=?1", [id])?;
        }
    }
    rebuild(&tx)?;
    tx.execute_batch("UPDATE catalog_state SET revision=revision+1; PRAGMA user_version=9;")?;
    tx.commit()?;
    Ok(())
}

struct Candidate {
    id: i64,
    tags: Tags,
    local_video: bool,
    clip_id: Option<String>,
}

fn key(title: &str, artists: &[String]) -> (String, Vec<String>) {
    let mut names: Vec<_> = artists
        .iter()
        .map(|name| name.trim().to_lowercase())
        .collect();
    names.retain(|name| !name.is_empty());
    names.sort_unstable();
    names.dedup();
    (
        title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase(),
        names,
    )
}

fn compatible(audio: &Candidate, video: &Candidate) -> bool {
    if let (Some(a), Some(b)) = (&audio.tags.isrc, &video.tags.isrc) {
        return a.eq_ignore_ascii_case(b);
    }
    if let (Some(a), Some(b)) = (audio.tags.album_name(), video.tags.album_name()) {
        if !a.eq_ignore_ascii_case(b) {
            return false;
        }
    }
    match (audio.tags.duration, video.tags.duration) {
        (Some(a), Some(b)) => (a - b).abs() <= 3.0,
        _ => false,
    }
}

pub(super) fn rebuild(tx: &Transaction<'_>) -> LibraryResult<()> {
    let mut groups = HashMap::<_, Vec<Candidate>>::new();
    let mut statement = tx.prepare(
        "SELECT t.id, t.path, t.metadata, y.metadata FROM track t
         LEFT JOIN external_download y ON y.track_id=t.id
         WHERE EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
         ORDER BY t.id",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id = row.get(0)?;
        let path: String = row.get(1)?;
        let mut tags: Tags = serde_json::from_str(&row.get::<_, String>(2)?)?;
        let downloaded: Option<String> = row.get(3)?;
        let local_video = Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
            && has_video(Path::new(&path));
        let clip_id = tags.external_video_id.clone();
        let normalized =
            crate::external_tools::music_metadata::normalize(&tags.name, &mut tags.artists);
        let identity = key(&normalized, &tags.artists);
        if identity.0.is_empty() || identity.1.is_empty() {
            continue;
        }
        // Music releases carry an album. Uploads without one can be clip counterparts.
        let clip_id = clip_id.filter(|_| {
            local_video
                || downloaded.as_ref().is_some_and(|data| {
                    serde_json::from_str::<super::external::DownloadTags>(data).is_ok_and(
                        |download| {
                            download
                                .album
                                .as_deref()
                                .is_none_or(|album| album.trim().is_empty())
                        },
                    )
                })
        });
        groups.entry(identity).or_default().push(Candidate {
            id,
            tags,
            local_video,
            clip_id,
        });
    }

    tx.execute("DELETE FROM track_variant", [])?;
    let mut insert = tx.prepare("INSERT INTO track_variant VALUES (?1, ?2, ?3, ?4)")?;
    for group in groups.values() {
        for video in group
            .iter()
            .filter(|item| item.local_video || item.clip_id.is_some())
        {
            let mut candidates = group.iter().filter(|audio| {
                !audio.local_video
                    && audio.clip_id.is_none()
                    && audio.id != video.id
                    && (video.local_video || audio.tags.external_video_id != video.clip_id)
                    && compatible(audio, video)
            });
            let Some(audio) = candidates.next() else {
                continue;
            };
            if candidates.next().is_none() {
                insert.execute(params![
                    video.id,
                    audio.id,
                    video.local_video,
                    video.clip_id
                ])?;
            }
        }
    }
    Ok(())
}

pub(super) fn audio_id(db: &Connection, id: i64) -> rusqlite::Result<i64> {
    Ok(db
        .query_row(
            "SELECT audio_id FROM track_variant WHERE variant_id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(id))
}

pub(super) const METADATA: &str = "json_patch(t.metadata, json_object(
    'clipVideoId', COALESCE((SELECT clip_id FROM track_variant WHERE audio_id=t.id AND clip_id IS NOT NULL ORDER BY variant_id LIMIT 1), json_extract(t.metadata, '$.clipVideoId')),
    'hasLocalVideo', json(CASE WHEN EXISTS (SELECT 1 FROM track_variant WHERE audio_id=t.id AND local_video) OR json_extract(t.metadata, '$.hasLocalVideo') THEN 'true' ELSE 'false' END)))";

pub(super) fn video_path(db: &Connection, audio: &Path) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT COALESCE((SELECT v.path FROM track_variant r JOIN track v ON v.id=r.variant_id
                 WHERE r.audio_id=a.id AND r.local_video ORDER BY v.id LIMIT 1), a.path)
         FROM track a WHERE a.path=?1",
        [audio.to_str()],
        |row| row.get(0),
    )
    .optional()
}

pub(crate) fn has_video(path: &Path) -> bool {
    fn inspect(file: &mut File, end: u64, parents: &[&[u8; 4]]) -> std::io::Result<bool> {
        while end.saturating_sub(file.stream_position()?) >= 8 {
            let start = file.stream_position()?;
            let mut header = [0; 8];
            file.read_exact(&mut header)?;
            let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
            let size = match size {
                0 => end - start,
                1 => {
                    let mut large = [0; 8];
                    file.read_exact(&mut large)?;
                    u64::from_be_bytes(large)
                }
                size => u64::from(size),
            };
            let payload = file.stream_position()?;
            let Some(next) = start
                .checked_add(size)
                .filter(|next| *next <= end && *next >= payload)
            else {
                return Ok(false);
            };
            let kind = &header[4..];
            match parents.split_first() {
                Some((parent, tail)) if kind == *parent => {
                    if inspect(file, next, tail)? {
                        return Ok(true);
                    }
                }
                None if kind == b"hdlr" && next.saturating_sub(file.stream_position()?) >= 12 => {
                    let mut handler = [0; 12];
                    file.read_exact(&mut handler)?;
                    if &handler[8..] == b"vide" {
                        return Ok(true);
                    }
                }
                _ => {}
            }
            file.seek(SeekFrom::Start(next))?;
        }
        Ok(false)
    }
    (|| {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        inspect(&mut file, len, &[b"moov", b"trak", b"mdia"])
    })()
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut result = ((data.len() + 8) as u32).to_be_bytes().to_vec();
        result.extend_from_slice(kind);
        result.extend_from_slice(data);
        result
    }

    #[test]
    fn pairs_an_actual_mp4_video_with_audio_by_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("clip.mp4");
        let handler = atom(b"hdlr", b"\0\0\0\0\0\0\0\0vide");
        std::fs::write(
            &video,
            atom(b"moov", &atom(b"trak", &atom(b"mdia", &handler))),
        )
        .unwrap();
        let database = directory.path().join("catalog");
        super::super::Library::new(database.clone()).unwrap();
        let mut db = super::super::connect(&database).unwrap();
        let tx = db.transaction().unwrap();
        let mut tags = Tags {
            name: "8 миля".into(),
            artists: vec!["madk1d".into(), "VILLIAN".into()],
            duration: Some(115.0),
            album: Some("8 миля".into()),
            ..Tags::default()
        };
        let audio = directory.path().join("audio.mp3");
        for (id, path) in [
            (1, &audio),
            (2, &video),
            (3, &directory.path().join("other.mp3")),
        ] {
            if id == 2 {
                tags.artists.reverse();
            }
            if id == 3 {
                tags.duration = Some(160.0);
            }
            tx.execute(
                "INSERT INTO track (id,path,stamp,size,metadata) VALUES (?1,?2,'',0,?3)",
                params![id, path.to_str(), serde_json::to_string(&tags).unwrap()],
            )
            .unwrap();
            tx.execute("INSERT INTO source VALUES ('root',?1)", [id])
                .unwrap();
        }
        rebuild(&tx).unwrap();
        assert_eq!(audio_id(&tx, 2).unwrap(), 1);
        assert_eq!(audio_id(&tx, 3).unwrap(), 3);
        assert_eq!(video_path(&tx, &audio).unwrap().as_deref(), video.to_str());
    }
}
