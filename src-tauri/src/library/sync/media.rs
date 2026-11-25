use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Instant, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    child_dir, inside_exchange, regular_file,
    transport::{Directory, Exchange},
    Config, Manifest, Report, MAX_COUNT,
};
use crate::{
    audio,
    library::{
        edits, index_track, mark_catalog_addition, queries, LibraryResult, ReadWarnings, Tags,
    },
};

const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Media {
    extension: String,
    bytes: u64,
    original: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shared_file: Option<Vec<String>>,
    metadata: Metadata,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
struct Metadata {
    title: String,
    artists: Vec<String>,
    album: Option<String>,
    album_artist: Option<String>,
    year: Option<u16>,
    disc: Option<u32>,
    number: Option<u32>,
    release_date: Option<String>,
    is_single: Option<bool>,
    is_ep: bool,
}

fn valid_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}

impl Metadata {
    fn from_tags(tags: Tags) -> Self {
        Self {
            title: tags.name,
            artists: tags.artists,
            album: tags.album,
            album_artist: tags.album_artist,
            year: tags.year,
            disc: tags.disc_number,
            number: tags.track_number,
            release_date: tags.release_date,
            is_single: tags.is_single,
            is_ep: tags.is_ep,
        }
    }

    fn apply(self, tags: &mut Tags) {
        tags.name = self.title;
        tags.artists = self.artists;
        tags.album = self.album;
        tags.album_artist = self.album_artist;
        tags.year = self.year;
        tags.disc_number = self.disc;
        tags.track_number = self.number;
        tags.release_date = self.release_date;
        tags.is_single = self.is_single;
        tags.is_ep = self.is_ep;
        tags.fallback_title = false;
    }

    fn valid(&self) -> bool {
        valid_text(&self.title, 4096)
            && self.artists.len() <= 96
            && self.artists.iter().all(|value| valid_text(value, 1024))
            && self
                .album
                .as_ref()
                .is_none_or(|value| valid_text(value, 4096))
            && self
                .album_artist
                .as_ref()
                .is_none_or(|value| valid_text(value, 4096))
            && self
                .release_date
                .as_ref()
                .is_none_or(|value| valid_text(value, 32))
    }
}

impl Media {
    pub(super) fn valid(&self) -> bool {
        matches!(
            self.extension.as_str(),
            "mp3" | "m4a" | "mp4" | "ogg" | "opus" | "webm" | "wav" | "flac"
        ) && (1..=MAX_AUDIO_BYTES).contains(&self.bytes)
            && self.metadata.valid()
            && self
                .shared_file
                .as_ref()
                .is_none_or(|parts| valid_shared_file(parts, &self.extension))
    }
}

fn valid_shared_file(parts: &[String], extension: &str) -> bool {
    !parts.is_empty()
        && parts.len() <= 64
        && parts.iter().all(|part| {
            valid_text(part, 255)
                && !matches!(part.as_str(), "." | "..")
                && !part.contains(['/', '\\', ':'])
                && !super::is_exchange_dir(part.as_ref())
        })
        && parts
            .last()
            .and_then(|name| Path::new(name).extension())
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

fn relative_shared_file(folder: &Path, path: &Path, extension: &str) -> Option<Vec<String>> {
    let parts = path
        .strip_prefix(folder)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    valid_shared_file(&parts, extension).then_some(parts)
}

pub(in crate::library) fn apply_imported_metadata(
    db: &Connection,
    path: &str,
    tags: &mut Tags,
) -> LibraryResult<()> {
    let metadata: Option<String> = db.query_row(
        "SELECT si.metadata FROM sync_import si JOIN track t ON t.id=si.track_id WHERE t.path=?1",
        [path], |row| row.get(0),
    ).optional()?;
    if let Some(metadata) = metadata {
        serde_json::from_str::<Metadata>(&metadata)?.apply(tags);
    }
    Ok(())
}

pub(super) struct Local {
    id: i64,
    path: String,
    stamp: String,
    size: i64,
    tags: String,
    original: bool,
    digest: Option<String>,
    needs_identity: bool,
    share: bool,
}

pub(super) fn read_local(
    db: &Connection,
    sources: &[String],
    peers: &[Manifest],
) -> LibraryResult<Vec<Local>> {
    let peer_sizes: HashSet<_> = peers
        .iter()
        .flat_map(|peer| peer.media.values().map(|media| media.bytes))
        .collect();
    let mut statement = db.prepare(
        "WITH selected AS (
             SELECT DISTINCT track_id FROM source WHERE root IN (SELECT value FROM json_each(?1))
         )
         SELECT t.id,t.path,t.stamp,t.size,t.metadata,si.track_id IS NULL,identity.digest,s.track_id IS NOT NULL
         FROM track t LEFT JOIN selected s ON s.track_id=t.id LEFT JOIN sync_identity identity
            ON identity.track_id=t.id AND identity.stamp=t.stamp AND identity.size=t.size
            AND identity.before_change IS NULL
         LEFT JOIN sync_import si ON si.track_id=t.id
         WHERE EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
          AND NOT EXISTS (SELECT 1 FROM track_variant WHERE variant_id=t.id)
          AND (s.track_id IS NOT NULL OR t.size IN (SELECT value FROM json_each(?2)))
         ORDER BY si.track_id IS NOT NULL,t.id",
    )?;
    let rows = statement.query_map(
        params![
            serde_json::to_string(sources)?,
            serde_json::to_string(&peer_sizes)?
        ],
        |row| {
            Ok(Local {
                id: row.get(0)?,
                path: row.get(1)?,
                stamp: row.get(2)?,
                size: row.get(3)?,
                tags: row.get(4)?,
                original: row.get(5)?,
                digest: row.get(6)?,
                needs_identity: false,
                share: row.get(7)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn digest_file(
    path: &Path,
    output: impl Write,
    cancelled: &AtomicBool,
) -> io::Result<(String, u64)> {
    digest_stream(regular_file(path, MAX_AUDIO_BYTES)?, output, cancelled)
}

fn digest_stream(
    source: impl Read,
    mut output: impl Write,
    cancelled: &AtomicBool,
) -> io::Result<(String, u64)> {
    let mut source = source.take(MAX_AUDIO_BYTES + 1);
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(io::Error::other("Sync cancelled"));
        }
        let read = source.read(&mut buffer)?;
        if read == 0 {
            return Ok((format!("{:x}", hasher.finalize()), bytes));
        }
        bytes += read as u64;
        if bytes > MAX_AUDIO_BYTES {
            return Err(io::Error::other("Media exceeds the sync size limit"));
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
}

fn validate_local(track: &Local, path: &Path) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    let stamp = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    if metadata.len() != track.size as u64 || stamp.as_nanos().to_string() != track.stamp {
        return Err(io::Error::other(
            "Media changed since the last library scan",
        ));
    }
    Ok(())
}

enum Publish {
    New,
    Repair,
}

fn copy_verified(
    source: impl Read,
    destination: &Path,
    digest: &str,
    size: u64,
    mode: Publish,
    cancelled: &AtomicBool,
) -> io::Result<u64> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("Missing track directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let actual = digest_stream(source, &mut temporary, cancelled)?;
    if actual.0 != digest || actual.1 != size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Media hash or size differs from the manifest",
        ));
    }
    temporary.as_file().sync_all()?;
    if matches!(mode, Publish::Repair) {
        temporary.persist(destination)?;
        return Ok(size);
    }
    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(size),
        Err(error)
            if error.error.kind() == io::ErrorKind::AlreadyExists
                && digest_file(destination, io::sink(), cancelled)? == actual =>
        {
            Ok(0)
        }
        Err(error) => Err(error.error),
    }
}

fn send_blob(
    source: &Path,
    digest: &str,
    size: u64,
    requested: bool,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<u64> {
    let destination = directory.tracks.join(digest);
    let bytes = match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.is_file() => {
            let same_size = metadata.len() == size;
            if same_size
                && (!requested
                    || digest_file(&destination, io::sink(), cancelled)?
                        == (digest.to_owned(), size))
            {
                0
            } else {
                copy_verified(
                    regular_file(source, MAX_AUDIO_BYTES)?,
                    &destination,
                    digest,
                    size,
                    Publish::Repair,
                    cancelled,
                )?
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => copy_verified(
            regular_file(source, MAX_AUDIO_BYTES)?,
            &destination,
            digest,
            size,
            Publish::New,
            cancelled,
        )?,
        Ok(_) => return Err(io::Error::other("Shared track path is not a regular file")),
        Err(error) => return Err(error),
    };
    if requested {
        let _ = fs::remove_file(directory.repairs.join(digest));
    }
    Ok(bytes)
}

pub(super) struct Incoming<'a> {
    digest: &'a str,
    metadata: &'a Metadata,
    path: Option<PathBuf>,
    in_exchange: bool,
}

#[derive(Default)]
struct Available {
    original: bool,
    orphan: Option<PathBuf>,
}

pub(super) fn retired(db: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut query = db.prepare("SELECT digest FROM sync_retired")?;
    let rows = query.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub(super) struct Transfer<'a> {
    pub roots: &'a Mutex<Vec<PathBuf>>,
    pub destination: &'a Path,
    pub exchange: &'a Exchange,
    pub exchange_is_library_root: bool,
    pub retired: &'a HashSet<String>,
    pub cancelled: &'a AtomicBool,
}

pub(super) fn exchange<'a>(
    tracks: &mut [Local],
    peers: &'a [Manifest],
    transfer: Transfer<'_>,
    report: &mut Report,
    progress: &mut impl FnMut(&'static str, usize, Option<usize>),
) -> (BTreeMap<String, Media>, Vec<Incoming<'a>>) {
    let total = tracks.len();
    progress("indexing", 0, Some(total));
    let mut available = HashMap::<String, Available>::with_capacity(total);
    let mut candidates = BTreeMap::new();
    let mut verified = 0usize;
    let receive_directory = transfer.destination.join("YAP Sync");
    for (index, track) in tracks.iter_mut().enumerate() {
        if transfer.cancelled.load(Ordering::Relaxed) {
            break;
        }
        let result = (|| -> LibraryResult<()> {
            let path = audio::library_track_path(&track.path, transfer.roots)?;
            if inside_exchange(&path) {
                return Ok(());
            }
            validate_local(track, &path)?;
            if track.digest.is_none() {
                let (digest, size) = digest_file(&path, io::sink(), transfer.cancelled)?;
                report.bytes_hashed += size;
                validate_local(track, &path)?;
                track.digest = Some(digest);
                track.needs_identity = true;
            }
            let digest = track.digest.as_deref().ok_or("Missing track identity")?;
            verified += 1;
            let managed_name = path.parent() == Some(receive_directory.as_path())
                && path.file_stem().and_then(|name| name.to_str()) == Some(digest);
            let original = track.original && !managed_name;
            let state = available
                .entry(digest.to_owned())
                .or_insert_with(Available::default);
            state.original |= original;
            if managed_name && track.original {
                state.orphan = Some(path.clone());
            }
            if !track.share {
                return Ok(());
            }
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let shared_file = match transfer.exchange {
                Exchange::Directory(directory) if original => {
                    relative_shared_file(&directory.root, &path, &extension)
                }
                _ => None,
            };
            let item = Media {
                extension,
                bytes: track.size as u64,
                original,
                shared_file,
                metadata: Metadata::from_tags(serde_json::from_str(&track.tags)?),
            };
            if !item.valid() {
                report.warn("trackMetadata");
                return Ok(());
            }
            match candidates.entry(digest.to_owned()) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let (_, previous): &(_, Media) = entry.get();
                    if original && !previous.original {
                        entry.insert((path, item));
                    }
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((path, item));
                }
            }
            Ok(())
        })();
        if result.is_err() {
            report.warn("localTrack");
        }
        progress("indexing", index + 1, Some(total));
    }
    report.tracks = available.len();
    report.duplicates = verified.saturating_sub(available.len());

    let mut requested = HashSet::new();
    if let Exchange::Directory(directory) = transfer.exchange {
        if let Ok(entries) = fs::read_dir(&directory.repairs) {
            for entry in entries.flatten() {
                if let Ok(digest) = entry.file_name().into_string() {
                    if candidates.contains_key(&digest)
                        && entry.file_type().is_ok_and(|kind| kind.is_file())
                    {
                        requested.insert(digest);
                    }
                }
            }
        }
    }
    let mut outgoing = BTreeMap::new();
    let total = candidates.len();
    progress("sending", 0, Some(total));
    let present = match transfer.exchange {
        Exchange::Server(server) => server
            .present(
                candidates
                    .iter()
                    .map(|(digest, (_, media))| (digest.as_str(), media.bytes)),
                transfer.cancelled,
            )
            .ok(),
        Exchange::Directory(_) => None,
    };
    for (index, (digest, (path, item))) in candidates.into_iter().enumerate() {
        if transfer.cancelled.load(Ordering::Relaxed) {
            break;
        }
        let started = Instant::now();
        let result = path
            .to_str()
            .and_then(|path| audio::library_track_path(path, transfer.roots).ok())
            .ok_or_else(|| io::Error::other("Track is no longer available"))
            .and_then(|source| match transfer.exchange {
                Exchange::Directory(directory) => send_blob(
                    &source,
                    &digest,
                    item.bytes,
                    requested.contains(&digest),
                    directory,
                    transfer.cancelled,
                ),
                Exchange::Server(_)
                    if present
                        .as_ref()
                        .is_some_and(|present| present.contains(&digest)) =>
                {
                    Ok(0)
                }
                Exchange::Server(server) => server.send(
                    &digest,
                    &source,
                    item.bytes,
                    present.is_none(),
                    transfer.cancelled,
                ),
            });
        match result {
            Ok(bytes) => {
                if bytes > 0 {
                    report.transfer_time += started.elapsed();
                    report.bytes_sent += bytes;
                }
                outgoing.insert(digest, item);
            }
            Err(_) => report.warn("sendTrack"),
        }
        progress("sending", index + 1, Some(total));
    }

    let mut selected: BTreeMap<&str, &Media> = BTreeMap::new();
    for peer in peers {
        for (digest, media) in &peer.media {
            match selected.entry(digest) {
                std::collections::btree_map::Entry::Occupied(mut entry)
                    if (media.original, media.shared_file.is_some())
                        > (entry.get().original, entry.get().shared_file.is_some()) =>
                {
                    entry.insert(media);
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(media);
                }
                _ => {}
            }
        }
    }
    let pending = selected
        .keys()
        .filter(|&&digest| !available.contains_key(digest) && !transfer.retired.contains(digest))
        .count();
    let mut completed = 0;
    progress("receiving", 0, Some(pending));
    let mut incoming = Vec::new();
    // Prefer a native library copy to a relayed copy when choosing the portable tags.
    for (digest, media) in selected {
        if transfer.cancelled.load(Ordering::Relaxed) {
            break;
        }
        if let Some(state) = available.get_mut(digest) {
            report.duplicates += 1;
            if !state.original && (media.original || state.orphan.is_some()) {
                incoming.push(Incoming {
                    digest,
                    metadata: &media.metadata,
                    path: state.orphan.take(),
                    in_exchange: false,
                });
            }
            continue;
        }
        if transfer.retired.contains(digest) {
            continue;
        }
        let shared_file = match (transfer.exchange, media.shared_file.as_ref()) {
            (Exchange::Directory(directory), Some(parts)) if transfer.exchange_is_library_root => {
                Some((&directory.root, parts))
            }
            _ => None,
        };
        let result = (|| -> io::Result<_> {
            if let Some((root, parts)) = shared_file {
                let path = root.join(parts.iter().collect::<PathBuf>());
                // The shared directory is already in the local library. Wait for the
                // original to arrive instead of creating another copy under the library root.
                if fs::canonicalize(&path)? != path
                    || audio::library_track_path(
                        path.to_str()
                            .ok_or_else(|| io::Error::other("Invalid shared file path"))?,
                        transfer.roots,
                    )
                    .is_err()
                    || digest_file(&path, io::sink(), transfer.cancelled)?
                        != (digest.to_owned(), media.bytes)
                {
                    return Err(io::Error::other(
                        "Shared source file is unavailable or differs from the manifest",
                    ));
                }
                return Ok(path);
            }
            if !transfer
                .roots
                .lock()
                .map_err(|_| io::Error::other("Library folders unavailable"))?
                .iter()
                .any(|root| root == transfer.destination)
            {
                return Err(io::Error::other("Destination is no longer registered"));
            }
            let directory = child_dir(transfer.destination, "YAP Sync")?;
            let path = directory.join(format!("{digest}.{}", media.extension));
            if path.try_exists()?
                && digest_file(&path, io::sink(), transfer.cancelled)?
                    == (digest.to_owned(), media.bytes)
            {
                return Ok(path);
            }
            let started = Instant::now();
            let bytes = match transfer.exchange {
                Exchange::Directory(directory) => copy_verified(
                    regular_file(&directory.tracks.join(digest), MAX_AUDIO_BYTES)?,
                    &path,
                    digest,
                    media.bytes,
                    Publish::New,
                    transfer.cancelled,
                ),
                Exchange::Server(server) => copy_verified(
                    server.receive(digest, media.bytes)?,
                    &path,
                    digest,
                    media.bytes,
                    Publish::New,
                    transfer.cancelled,
                ),
            }?;

            if bytes > 0 {
                report.transfer_time += started.elapsed();
                report.bytes_received += bytes;
            }
            Ok(path)
        })();
        match result {
            Ok(path) => incoming.push(Incoming {
                digest,
                metadata: &media.metadata,
                path: Some(path),
                in_exchange: shared_file.is_some(),
            }),
            Err(error) => {
                if error.kind() == io::ErrorKind::InvalidData {
                    transfer.exchange.signal_repair(digest);
                }
                report.warn("receiveTrack");
            }
        }
        completed += 1;
        progress("receiving", completed, Some(pending));
    }
    (outgoing, incoming)
}

fn update_metadata(
    db: &Transaction<'_>,
    id: i64,
    digest: &str,
    path: &str,
    metadata: &Metadata,
) -> LibraryResult<bool> {
    let json = serde_json::to_string(metadata)?;
    if db.execute(
        "INSERT INTO sync_import VALUES (?1,?2,?3)
         ON CONFLICT(track_id) DO UPDATE SET digest=excluded.digest,metadata=excluded.metadata
         WHERE digest<>excluded.digest OR metadata<>excluded.metadata",
        params![id, digest, json],
    )? == 0
    {
        return Ok(false);
    }
    let tags: String = db.query_row("SELECT metadata FROM track WHERE id=?1", [id], |row| {
        row.get(0)
    })?;
    let mut tags: Tags = serde_json::from_str(&tags)?;
    metadata.clone().apply(&mut tags);
    edits::apply(db, path, &mut tags)?;
    db.execute(
        "UPDATE track SET metadata=?2 WHERE id=?1",
        params![id, serde_json::to_string(&tags)?],
    )?;
    queries::index(db, id, path, &tags)?;
    Ok(true)
}

fn save_identity(db: &Connection, track: i64, digest: &str) -> rusqlite::Result<bool> {
    let replacement = db.prepare_cached(
        "SELECT i.digest, COALESCE(i.before_change,t.play_count-COALESCE(p.extra,0)), COALESCE(p.extra,0)
         FROM sync_identity i JOIN track t ON t.id=i.track_id
         LEFT JOIN sync_play_projection p ON p.track_id=i.track_id
         WHERE t.id=?1 AND i.digest<>?2",
    )?.query_row(params![track, digest], |row| Ok((
        row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?,
    ))).optional()?;
    if let Some((ref old, before_change, extra)) = replacement {
        if before_change > 0 {
            db.prepare_cached(
                "INSERT INTO sync_detached_plays VALUES (?1,min(?2,?3))
                 ON CONFLICT(digest) DO UPDATE SET plays=min(?3,plays+excluded.plays)",
            )?
            .execute(params![old, before_change, MAX_COUNT])?;
        }
        db.prepare_cached("UPDATE track SET play_count=max(0,play_count-?2-?3) WHERE id=?1")?
            .execute(params![track, before_change, extra])?;
        db.prepare_cached("DELETE FROM sync_play_projection WHERE track_id=?1")?
            .execute([track])?;
    }
    db.prepare_cached(
        "INSERT INTO sync_identity (track_id,stamp,size,digest)
         SELECT id,stamp,size,?2 FROM track WHERE id=?1
         ON CONFLICT(track_id) DO UPDATE SET stamp=excluded.stamp,size=excluded.size,digest=excluded.digest,
           before_change=NULL
         WHERE stamp<>excluded.stamp OR size<>excluded.size OR digest<>excluded.digest OR before_change IS NOT NULL",
    )?.execute(params![track, digest])?;
    Ok(replacement.is_some())
}

pub(super) fn apply(
    db: &Transaction<'_>,
    tracks: &[Local],
    incoming: &[Incoming<'_>],
    config: &Config,
) -> LibraryResult<(bool, usize)> {
    let mut changed = false;
    {
        let mut current = db.prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM track WHERE id=?1 AND stamp=?2 AND size=?3)",
        )?;
        for track in tracks.iter().filter(|track| track.needs_identity) {
            if let Some(digest) = &track.digest {
                let matches: bool = current
                    .query_row(params![track.id, track.stamp, track.size], |row| row.get(0))?;
                if matches {
                    changed |= save_identity(db, track.id, digest)?;
                }
            }
        }
    }
    let mut imported = 0;
    for item in incoming {
        if let Some(path) = &item.path {
            let text = path.to_str().ok_or("Sync destination is not UTF-8")?;
            let id = index_track(db, path, None, &mut ReadWarnings::default())?;
            changed |= save_identity(db, id, item.digest)?;
            mark_catalog_addition(db, id)?;
            let source = if item.in_exchange {
                &config.folder
            } else {
                &config.destination
            };
            let added = db.execute(
                "INSERT OR IGNORE INTO source VALUES (?1,?2)",
                params![source, id],
            )? > 0;
            imported += usize::from(added);
            changed |= update_metadata(db, id, item.digest, text, item.metadata)? || added;
        } else {
            let mut query = db.prepare_cached(
                "SELECT t.id,t.path FROM sync_import si JOIN track t ON si.track_id=t.id
                 JOIN sync_identity identity ON identity.track_id=t.id
                 WHERE identity.digest=?1 AND identity.stamp=t.stamp AND identity.size=t.size",
            )?;
            let locals = query.query_map([item.digest], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            for local in locals {
                let (id, path) = local?;
                changed |= update_metadata(db, id, item.digest, &path, item.metadata)?;
            }
        }
    }
    Ok((changed, imported))
}
