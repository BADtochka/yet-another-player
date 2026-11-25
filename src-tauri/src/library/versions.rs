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
    explicit_clip: bool,
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
        let same_release = super::catalog::release_title(a).1 == super::catalog::release_title(b).1
            && super::catalog::release_key(a) == super::catalog::release_key(b);
        let standalone_clip = video.explicit_clip
            && super::catalog::release_key(b) == super::catalog::release_key(&video.tags.name);
        if !same_release && !standalone_clip {
            return false;
        }
    }
    match (audio.tags.duration, video.tags.duration) {
        (Some(a), Some(b)) if a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0 => {
            let tolerance = if video.explicit_clip {
                (a * 0.2).clamp(3.0, 30.0)
            } else {
                3.0
            };
            (a - b).abs() <= tolerance
        }
        _ => false,
    }
}

struct AlbumMember {
    id: i64,
    name: String,
    artist: String,
    explicit_artist: bool,
    year: Option<u16>,
}

fn group_albums(
    tx: &Transaction<'_>,
    albums: HashMap<(String, super::catalog::ReleaseKind, String), Vec<AlbumMember>>,
) -> LibraryResult<()> {
    let mut update = tx.prepare(
        "UPDATE track_index SET album_id=?2, album_name=?3, album_artist=?4
         WHERE id=?1 AND (album_id<>?2 OR album_name<>?3 OR album_artist<>?4)",
    )?;
    let has_artwork = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='release_artwork')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    for members in albums.values() {
        let year = members.iter().find_map(|member| member.year);
        let conflict = members
            .iter()
            .filter_map(|member| member.year)
            .any(|candidate| Some(candidate) != year);
        let canonical = members
            .iter()
            .min_by_key(|member| (member.name.len(), member.id));
        let credit = members
            .iter()
            .min_by_key(|member| (!member.explicit_artist, member.artist.len(), member.id));
        for member in members {
            let display = canonical.filter(|_| !conflict).unwrap_or(member);
            let artist = credit.filter(|_| !conflict).unwrap_or(member);
            let key = serde_json::to_string(&[&display.name, &display.artist])?;
            let changed = update.execute(params![member.id, key, display.name, artist.artist])?;
            if changed > 0 && has_artwork {
                let previous = serde_json::to_string(&[&member.name, &member.artist])?;
                if previous != key {
                    tx.execute(
                        "INSERT OR IGNORE INTO release_artwork (album_id, url)
                         SELECT ?1, url FROM release_artwork WHERE album_id=?2",
                        params![key, previous],
                    )?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn rebuild(tx: &Transaction<'_>) -> LibraryResult<()> {
    let mut groups = HashMap::<_, Vec<Candidate>>::new();
    let mut albums = HashMap::<_, Vec<AlbumMember>>::new();
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
        let downloaded = row
            .get::<_, Option<String>>(3)?
            .and_then(|data| serde_json::from_str::<super::external::DownloadTags>(&data).ok());
        if let Some(name) = tags.album_name() {
            let explicit = tags
                .album_artist
                .as_deref()
                .map(str::trim)
                .filter(|credit| !credit.is_empty());
            let artist = explicit
                .map(str::to_owned)
                .unwrap_or_else(|| tags.artists.join(", "));
            let lead = artist
                .split([',', ';'])
                .next()
                .unwrap_or("")
                .split(" & ")
                .next()
                .unwrap_or("");
            let lead = super::catalog::normalized(lead);
            let key = super::catalog::release_key(name);
            if !lead.is_empty() && !key.is_empty() {
                albums
                    .entry((key, super::catalog::release_title(name).1, lead))
                    .or_default()
                    .push(AlbumMember {
                        id,
                        name: name.to_owned(),
                        artist,
                        explicit_artist: explicit.is_some(),
                        year: tags.year,
                    });
            }
        }
        let local_video = tags.has_local_video
            || (Path::new(&path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
                && has_video(Path::new(&path)));
        let explicit_clip = local_video
            || tags.clip_video_id.is_some()
            || downloaded.as_ref().is_some_and(|download| download.is_clip)
            || crate::external_tools::music_metadata::has_clip_label(&tags.name)
            || Path::new(&path)
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(crate::external_tools::music_metadata::has_clip_label);
        let clip_id = tags
            .clip_video_id
            .clone()
            .or_else(|| tags.external_video_id.clone())
            .filter(|_| {
                explicit_clip
                    || downloaded.as_ref().is_some_and(|download| {
                        download
                            .album
                            .as_deref()
                            .is_none_or(|album| album.trim().is_empty())
                    })
            });
        let normalized =
            crate::external_tools::music_metadata::normalize(&tags.name, &mut tags.artists);
        let identity = key(&normalized, &tags.artists);
        if identity.0.is_empty() || identity.1.is_empty() {
            continue;
        }
        groups.entry(identity).or_default().push(Candidate {
            id,
            tags,
            local_video,
            explicit_clip,
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
                !audio.explicit_clip
                    && (audio.clip_id.is_none()
                        || (video.explicit_clip && audio.tags.album_name().is_some()))
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
    group_albums(tx, albums)
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

pub(super) const PLAY_COUNT: &str = "t.play_count + COALESCE((
    SELECT SUM(variant.play_count) FROM track_variant r
    JOIN track variant ON variant.id=r.variant_id WHERE r.audio_id=t.id
), 0)";

pub(super) const METADATA: &str = "json_patch(t.metadata, json_object(
    'catalogAlbumId', (SELECT album_id FROM track_index WHERE id=t.id),
    'album', CASE WHEN json_extract(t.metadata, '$.album') IS NOT NULL THEN COALESCE((SELECT album_name FROM track_index WHERE id=t.id), json_extract(t.metadata, '$.album')) END,
    'albumArtist', CASE WHEN json_extract(t.metadata, '$.album') IS NOT NULL THEN COALESCE((SELECT album_artist FROM track_index WHERE id=t.id), json_extract(t.metadata, '$.albumArtist')) ELSE json_extract(t.metadata, '$.albumArtist') END,
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
    fn upgrades_split_album_and_links_the_longer_clip_to_its_audio_track() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("catalog");
        let library = super::super::Library::new(database.clone()).unwrap();
        let mut db = super::super::connect(&database).unwrap();
        let tx = db.transaction().unwrap();
        let audio = Tags {
            name: "ДИНАСТИЯ".into(),
            artists: vec!["VILLIAN".into(), "madk1d".into()],
            album: Some("Красивое Зло".into()),
            track_number: Some(5),
            duration: Some(150.456),
            external_video_id: Some("p-LBiCtzXu0".into()),
            isrc: Some("RUAGW2626371".into()),
            year: Some(2026),
            ..Tags::default()
        };
        let clip = Tags {
            album: Some("Красивое Зло (Beautiful Evil)".into()),
            album_artist: Some("VILLIAN".into()),
            clip_video_id: Some("J7NFL-eOxiQ".into()),
            external_video_id: Some("J7NFL-eOxiQ".into()),
            duration: Some(164.881),
            name: audio.name.clone(),
            artists: audio.artists.clone(),
            year: audio.year,
            ..Tags::default()
        };
        let other = Tags {
            name: "Общество Мертвых Поэтов".into(),
            artists: vec!["VILLIAN".into(), "Aarne".into()],
            album: clip.album.clone(),
            album_artist: clip.album_artist.clone(),
            clip_video_id: Some("dqK-S9e9720".into()),
            duration: Some(127.061),
            year: clip.year,
            ..Tags::default()
        };
        for (id, path, tags, count) in [
            (1, "/music/audio.mp3", audio, 2),
            (2, "/music/clip.webm", clip, 3),
            (3, "/music/other.webm", other, 0),
        ] {
            tx.execute(
                "INSERT INTO track (id,path,stamp,size,metadata,play_count) VALUES (?1,?2,'',0,?3,?4)",
                params![id, path, serde_json::to_string(&tags).unwrap(), count],
            ).unwrap();
            super::super::queries::index(&tx, id, path, &tags).unwrap();
            tx.execute("INSERT INTO source VALUES ('root',?1)", [id])
                .unwrap();
        }
        tx.execute_batch("PRAGMA user_version=15;").unwrap();
        tx.commit().unwrap();
        drop(db);
        drop(library);

        let library = super::super::Library::new(database.clone()).unwrap();
        let db = super::super::connect(&database).unwrap();
        assert_eq!(audio_id(&db, 2).unwrap(), 1);
        assert!(library
            .owns_clip(Path::new("/music/audio.mp3"), "J7NFL-eOxiQ")
            .unwrap());
        assert!(!library
            .owns_clip(Path::new("/music/audio.mp3"), "another-id")
            .unwrap());

        let mut query = db.prepare(&format!(
            "SELECT {}, {} FROM track t WHERE id NOT IN (SELECT variant_id FROM track_variant) ORDER BY id",
            METADATA, PLAY_COUNT
        )).unwrap();
        let mut rows = query.query([]).unwrap();
        let first = rows.next().unwrap().unwrap();
        let metadata: String = first.get(0).unwrap();
        let audio: Tags = serde_json::from_str(&metadata).unwrap();
        assert_eq!(audio.clip_video_id.as_deref(), Some("J7NFL-eOxiQ"));
        assert_eq!(first.get::<_, i64>(1).unwrap(), 5);
        let other: Tags =
            serde_json::from_str(&rows.next().unwrap().unwrap().get::<_, String>(0).unwrap())
                .unwrap();
        assert_eq!(audio.catalog_album_id, other.catalog_album_id);
        assert_eq!(other.album.as_deref(), Some("Красивое Зло"));
        assert!(rows.next().unwrap().is_none());
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
