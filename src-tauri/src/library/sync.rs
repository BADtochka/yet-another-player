pub(crate) mod discovery;
mod media;
mod metadata;
mod playlist;
pub(crate) mod transport;
use transport::{Access, DeviceAuth, Exchange, Invite, Server};
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rand::TryRngCore;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};

use super::{connect, versions, Library, LibraryResult};
use crate::MusicFolders;

const VERSION: u32 = 1;
const EXCHANGE_DIR: &str = "YAP-Sync-v1";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PEER_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PEERS: usize = 128;
const MAX_CLOCK: i64 = 9_000_000_000_000_000;
const MAX_COUNT: i64 = 1_000_000_000;

pub(super) fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_state (
            device TEXT NOT NULL, clock INTEGER NOT NULL DEFAULT 0,
            config TEXT, report TEXT
         );
         INSERT INTO sync_state (device) SELECT lower(hex(randomblob(16)))
         WHERE NOT EXISTS (SELECT 1 FROM sync_state);
         CREATE TABLE IF NOT EXISTS sync_identity (
            track_id INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
            stamp TEXT NOT NULL, size INTEGER NOT NULL, digest TEXT NOT NULL,
            before_change INTEGER
         );
         CREATE INDEX IF NOT EXISTS sync_identity_digest ON sync_identity(digest);
         CREATE TABLE IF NOT EXISTS sync_import (
            track_id INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
            digest TEXT NOT NULL, metadata TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS sync_retired (digest TEXT PRIMARY KEY);
         CREATE TRIGGER IF NOT EXISTS sync_retire_track AFTER INSERT ON track_trash BEGIN
            INSERT OR IGNORE INTO sync_retired
            SELECT digest FROM sync_identity WHERE track_id=new.track_id AND stamp=new.stamp AND size=new.size;
         END;
         CREATE TABLE IF NOT EXISTS sync_counter (
            device TEXT NOT NULL, digest TEXT NOT NULL, plays INTEGER NOT NULL,
            PRIMARY KEY (digest, device)
         );
         CREATE TABLE IF NOT EXISTS sync_detached_plays (digest TEXT PRIMARY KEY, plays INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS sync_play_projection (
            track_id INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
            extra INTEGER NOT NULL
         );
         CREATE TRIGGER IF NOT EXISTS sync_note_track_change
         AFTER UPDATE OF stamp,size ON track
         WHEN old.stamp<>new.stamp OR old.size<>new.size BEGIN
            UPDATE sync_identity SET before_change=old.play_count-
                COALESCE((SELECT extra FROM sync_play_projection WHERE track_id=old.id),0)
            WHERE track_id=old.id AND before_change IS NULL;
         END;
         CREATE TABLE IF NOT EXISTS sync_playlist (
            key TEXT PRIMARY KEY, local_id INTEGER UNIQUE, baseline TEXT NOT NULL DEFAULT '{}'
         );
         CREATE TABLE IF NOT EXISTS sync_register (
            playlist TEXT NOT NULL, field TEXT NOT NULL, clock INTEGER NOT NULL,
            device TEXT NOT NULL, value TEXT NOT NULL,
            PRIMARY KEY (playlist, field)
         );
         CREATE INDEX IF NOT EXISTS sync_register_device ON sync_register(device);
         CREATE TABLE IF NOT EXISTS sync_metadata (
            digest TEXT NOT NULL, field TEXT NOT NULL, clock INTEGER NOT NULL,
            device TEXT NOT NULL, value TEXT NOT NULL,
            PRIMARY KEY (digest,field)
         );
         CREATE INDEX IF NOT EXISTS sync_metadata_device ON sync_metadata(device);
         CREATE TABLE IF NOT EXISTS sync_pending_metadata (
            track_id INTEGER NOT NULL REFERENCES track(id) ON DELETE CASCADE,
            field TEXT NOT NULL, clock INTEGER NOT NULL, value TEXT NOT NULL,
            PRIMARY KEY (track_id,field)
         );
         INSERT OR IGNORE INTO sync_pending_metadata
         SELECT e.track_id,j.key,1,e.fields -> j.key FROM track_edits e, json_each(e.fields) j
         WHERE j.key IN ('name','artists','album','albumArtist','releaseDate','genres','trackNumber','discNumber');
         UPDATE sync_state SET clock=1
         WHERE clock=0 AND EXISTS (SELECT 1 FROM sync_pending_metadata);
         PRAGMA user_version=19;",
    )?;
    tx.commit()
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    folder: String,
    destination: String,
    #[serde(default)]
    sources: Option<Vec<String>>,
    automatic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invite: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Selection {
    folder: String,
    automatic: bool,
    invite: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Status {
    config: Option<Config>,
    report: Option<Report>,
    address: Option<std::net::SocketAddr>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    finished_at: i64,
    elapsed_ms: u64,
    peers: usize,
    tracks: usize,
    duplicates: usize,
    imported: usize,
    playlists: usize,
    bytes_hashed: u64,
    bytes_sent: u64,
    bytes_received: u64,
    transfer_ms: u64,
    #[serde(skip)]
    transfer_time: Duration,
    catalog_changed: bool,
    manifest_bytes: usize,
    warnings: BTreeMap<String, usize>,
}

impl Report {
    fn warn(&mut self, key: &str) {
        *self.warnings.entry(key.into()).or_default() += 1;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    phase: &'static str,
    current: usize,
    total: Option<usize>,
}

struct ProgressReporter {
    channel: Channel<Progress>,
    phase: &'static str,
    last: Instant,
}

impl ProgressReporter {
    fn update(&mut self, phase: &'static str, current: usize, total: Option<usize>) {
        if self.phase != phase
            || total == Some(current)
            || self.last.elapsed() >= Duration::from_millis(80)
        {
            self.phase = phase;
            self.last = Instant::now();
            let _ = self.channel.send(Progress {
                phase,
                current,
                total,
            });
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Manifest {
    version: u32,
    device: String,
    media: BTreeMap<String, media::Media>,
    counts: BTreeMap<String, i64>,
    playlists: Vec<playlist::Register>,
    #[serde(default)]
    metadata_edits: Vec<metadata::Edit>,
}

impl Manifest {
    fn valid(&self, device: &str) -> bool {
        self.version == VERSION
            && self.device == device
            && self
                .media
                .iter()
                .all(|(key, media)| digest_valid(key) && media.valid())
            && self
                .counts
                .iter()
                .all(|(key, count)| digest_valid(key) && (1..=MAX_COUNT).contains(count))
            && self.playlists.iter().all(playlist::Register::valid)
            && self.metadata_edits.iter().all(metadata::Edit::valid)
    }
}

fn hex_valid(input: &str, len: usize) -> bool {
    input.len() == len
        && input
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn digest_valid(input: &str) -> bool {
    hex_valid(input, 64)
}

fn read_file(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    regular_file(path, limit)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("Sync file exceeds the size limit"));
    }
    Ok(bytes)
}

fn regular_file(path: &Path, limit: u64) -> io::Result<fs::File> {
    let entry = fs::symlink_metadata(path)?;
    if !entry.is_file() || entry.len() > limit {
        return Err(io::Error::other(
            "Sync file is not a regular file within the size limit",
        ));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    if fs::canonicalize(path)? != path {
        return Err(io::Error::other(
            "Sync file resolves outside its expected path",
        ));
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.len() > limit {
        return Err(io::Error::other("Sync file changed while opening"));
    }
    Ok(file)
}

fn child_dir(root: &Path, name: &str) -> io::Result<PathBuf> {
    let child = root.join(name);
    match fs::create_dir(&child) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    if fs::canonicalize(&child)? != child || !fs::symlink_metadata(&child)?.is_dir() {
        return Err(io::Error::other(
            "Sync directory is a symlink or outside its parent",
        ));
    }
    Ok(child)
}

fn read_status(db: &Connection) -> LibraryResult<Status> {
    let (config, report): (Option<String>, Option<String>) =
        db.query_row("SELECT config,report FROM sync_state", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    let config: Option<Config> = config.as_deref().map(serde_json::from_str).transpose()?;
    let address = config.as_ref().and_then(|config| {
        Invite::parse(config.invite.as_deref()?)
            .ok()
            .map(|invite| invite.address)
    });
    Ok(Status {
        config,
        report: report.as_deref().map(serde_json::from_str).transpose()?,
        address,
    })
}

fn read_peers(exchange: &Exchange, own: &str, report: &mut Report) -> io::Result<Vec<Manifest>> {
    let mut peers = Vec::new();
    let mut remaining = MAX_PEER_BYTES;
    for device in exchange.list()?.into_iter().filter(|id| id != own) {
        let manifest = exchange
            .manifest(&device, MAX_MANIFEST_BYTES.min(remaining))
            .ok()
            .flatten()
            .and_then(|bytes| {
                remaining -= bytes.len() as u64;
                serde_json::from_slice::<Manifest>(&bytes).ok()
            })
            .filter(|manifest| manifest.valid(&device));
        match manifest {
            Some(manifest) => peers.push(manifest),
            None => report.warn("peerManifest"),
        }
    }
    Ok(peers)
}

fn publish(
    exchange: &Exchange,
    manifest: &Manifest,
    previous: Option<&[u8]>,
) -> LibraryResult<usize> {
    let mut data = Vec::new();
    serde_json::to_writer(BoundedWriter { data: &mut data }, manifest)?;
    Ok(exchange.publish(&manifest.device, data, previous)?)
}

struct BoundedWriter<'a> {
    data: &'a mut Vec<u8>,
}

impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.data.len().saturating_add(bytes.len()) as u64 > MAX_MANIFEST_BYTES {
            return Err(io::Error::other("Sync manifest exceeds the size limit"));
        }
        self.data.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn counter_merge(
    db: &Connection,
    device: &str,
    counts: &BTreeMap<String, i64>,
) -> rusqlite::Result<()> {
    let mut statement = db.prepare_cached(
        "INSERT INTO sync_counter VALUES (?1,?2,?3)
         ON CONFLICT(digest,device) DO UPDATE SET plays=excluded.plays WHERE excluded.plays>plays",
    )?;
    for (digest, plays) in counts {
        statement.execute(params![device, digest, plays])?;
    }
    Ok(())
}

fn project_counts(db: &Connection, device: &str) -> LibraryResult<(BTreeMap<String, i64>, bool)> {
    // The visible counter includes the projection. Subtract it before reading this device's own plays.
    db.execute(
        "WITH contributions AS (
            SELECT identity.digest,
                   SUM(COALESCE(own.before_change,t.play_count-COALESCE(previous.extra,0))) AS plays
            FROM track t LEFT JOIN track_variant v ON v.variant_id=t.id
            JOIN sync_identity identity ON identity.track_id=COALESCE(v.audio_id,t.id)
            LEFT JOIN sync_play_projection previous ON previous.track_id=t.id
            LEFT JOIN sync_identity own ON own.track_id=t.id
            GROUP BY identity.digest
            UNION ALL
            SELECT digest,plays FROM sync_detached_plays
         )
         INSERT INTO sync_counter
         SELECT ?1,digest,min(?2,SUM(plays))
         FROM contributions GROUP BY digest HAVING SUM(plays)>0
         ON CONFLICT(digest,device) DO UPDATE SET plays=excluded.plays WHERE excluded.plays>plays",
        params![device, MAX_COUNT],
    )?;
    db.execute_batch(
        "CREATE TEMP TABLE sync_projection_next (track_id INTEGER PRIMARY KEY, extra INTEGER NOT NULL);
         WITH totals AS (
           SELECT digest, SUM(plays) AS plays FROM sync_counter GROUP BY digest
         ), local AS (
           SELECT COALESCE(v.audio_id,t.id) AS track_id,
                  SUM(COALESCE(own.before_change,t.play_count-COALESCE(previous.extra,0))) AS plays
           FROM track t LEFT JOIN track_variant v ON v.variant_id=t.id
           LEFT JOIN sync_play_projection previous ON previous.track_id=t.id
           LEFT JOIN sync_identity own ON own.track_id=t.id
           WHERE COALESCE(v.audio_id,t.id) IN (SELECT track_id FROM sync_identity)
           GROUP BY COALESCE(v.audio_id,t.id)
         )
         INSERT INTO sync_projection_next
         SELECT l.track_id, totals.plays-l.plays FROM local l
         JOIN sync_identity identity ON identity.track_id=l.track_id
         JOIN totals ON totals.digest=identity.digest WHERE totals.plays>l.plays;",
    )?;
    let changed = db.execute(
        "UPDATE track SET play_count=play_count
          - COALESCE((SELECT extra FROM sync_play_projection WHERE track_id=track.id),0)
          + COALESCE((SELECT extra FROM sync_projection_next WHERE track_id=track.id),0)
         WHERE id IN (
             SELECT track_id FROM sync_play_projection p WHERE extra IS NOT
               (SELECT extra FROM sync_projection_next n WHERE n.track_id=p.track_id)
             UNION
             SELECT track_id FROM sync_projection_next n WHERE NOT EXISTS (
               SELECT 1 FROM sync_play_projection p WHERE p.track_id=n.track_id
             )
         )",
        [],
    )? > 0;
    db.execute_batch(
        "DELETE FROM sync_play_projection WHERE track_id NOT IN (SELECT track_id FROM sync_projection_next);
         INSERT INTO sync_play_projection SELECT * FROM sync_projection_next WHERE true
         ON CONFLICT(track_id) DO UPDATE SET extra=excluded.extra WHERE extra<>excluded.extra;
         DROP TABLE sync_projection_next;",
    )?;
    let mut query = db.prepare("SELECT digest, plays FROM sync_counter WHERE device=?1")?;
    let rows = query.query_map([device], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok((rows.collect::<rusqlite::Result<_>>()?, changed))
}

fn classify(
    error: Box<dyn std::error::Error + Send + Sync>,
    code: &'static str,
    message: &'static str,
) -> Failure {
    eprintln!("Library sync ({code}): {error}");
    Failure { code, message }
}

#[derive(Serialize)]
pub(crate) struct Failure {
    code: &'static str,
    message: &'static str,
}

const CANCELLED: Failure = Failure {
    code: "cancelled",
    message: "Синхронизация остановлена.",
};
const BUSY: Failure = Failure {
    code: "busy",
    message: "Синхронизация уже идёт.",
};
const STORAGE: Failure = Failure {
    code: "storage",
    message: "Не удалось прочитать или сохранить данные синхронизации.",
};
const FOLDER: Failure = Failure {
    code: "folder",
    message: "Общая папка недоступна. Проверьте подключение и разрешения.",
};
const DESTINATION: Failure = Failure {
    code: "destination",
    message: "Выберите доступную папку медиатеки для полученных треков.",
};

fn storage_error(error: Box<dyn std::error::Error + Send + Sync>) -> Failure {
    classify(error, STORAGE.code, STORAGE.message)
}

fn folder_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Failure {
    classify(error.into(), FOLDER.code, FOLDER.message)
}

const INVALID_INVITE: Failure = Failure {
    code: "invite",
    message: "Найдите сервер YAP Sync в локальной сети и выберите его снова.",
};
const ACCESS_PENDING: Failure = Failure {
    code: "pairingPending",
    message: "Запрос отправлен. Разрешите подключение на компьютере с сервером.",
};
const ACCESS_BLOCKED: Failure = Failure {
    code: "pairingBlocked",
    message: "Доступ устройства закрыт в списке сервера.",
};
const SERVER_UPGRADE: Failure = Failure {
    code: "serverUpgrade",
    message: "Обновите сервер YAP Sync до версии со списком доступа.",
};
const SERVER_UNAVAILABLE: Failure = Failure {
    code: "server",
    message: "Нет связи с выбранным сервером. Проверьте локальную сеть и состояние сервера.",
};

fn server_error(error: io::Error) -> Failure {
    classify(
        error.into(),
        SERVER_UNAVAILABLE.code,
        SERVER_UNAVAILABLE.message,
    )
}

const EXCHANGE_FOLDER: Failure = Failure {
    code: "exchangeFolder",
    message: "Выберите папку обмена из списка папок медиатеки.",
};

const SOURCES: Failure = Failure {
    code: "sources",
    message: "Выберите папки для отправки из списка папок медиатеки.",
};

pub(super) fn is_exchange_dir(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(EXCHANGE_DIR))
}

pub(super) fn inside_exchange(path: &Path) -> bool {
    path.components()
        .any(|part| is_exchange_dir(part.as_os_str()))
}

fn resolve_folders<'a>(
    config: &'a Config,
    configured: &'a [String],
) -> Result<(PathBuf, PathBuf, &'a [String]), Failure> {
    let sources = if config.invite.is_some() {
        std::slice::from_ref(&config.destination)
    } else {
        config.sources.as_deref().unwrap_or(configured)
    };
    if sources.is_empty() || sources.iter().any(|source| !configured.contains(source)) {
        return Err(SOURCES);
    }
    if !configured.contains(&config.destination) {
        return Err(DESTINATION);
    }
    let destination = fs::canonicalize(&config.destination).map_err(|_| DESTINATION)?;
    if !destination.is_dir() || inside_exchange(&destination) {
        return Err(DESTINATION);
    }
    if config.invite.is_some() {
        return Ok((destination.clone(), destination, sources));
    }
    let folder = fs::canonicalize(&config.folder).map_err(folder_error)?;
    if !folder.is_dir() || inside_exchange(&folder) {
        return Err(FOLDER);
    }
    Ok((folder, destination, sources))
}

fn configured_roots(folders: &MusicFolders) -> Result<Vec<String>, Failure> {
    folders
        .configured
        .lock()
        .map(|items| items.clone())
        .map_err(|_| STORAGE)
}

fn load_device(database: &Path, id: String) -> LibraryResult<DeviceAuth> {
    let data = fs::canonicalize(database.parent().ok_or("App data is unavailable")?)?;
    let key_file = data.join("sync-device-key");
    let read_key = || -> LibraryResult<String> {
        let key = String::from_utf8(read_file(&key_file, 64)?)?;
        if !hex_valid(&key, 64) {
            return Err("Invalid sync device key".into());
        }
        Ok(key)
    };
    let key = if key_file.symlink_metadata().is_ok() {
        read_key()?
    } else {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.try_fill_bytes(&mut bytes)?;
        let key = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut file = tempfile::NamedTempFile::new_in(&data)?;
        file.write_all(key.as_bytes())?;
        file.as_file().sync_all()?;
        match file.persist_noclobber(&key_file) {
            Ok(_) => key,
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => read_key()?,
            Err(error) => return Err(error.into()),
        }
    };
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 120 && !name.chars().any(char::is_control)
        })
        .unwrap_or_else(|| format!("YAP {}", &id[..8]));
    Ok(DeviceAuth { id, key, name })
}

pub(crate) fn device_auth(database: &Path) -> LibraryResult<DeviceAuth> {
    let db = connect(database)?;
    let id = db.query_row("SELECT device FROM sync_state", [], |row| row.get(0))?;
    load_device(database, id)
}

#[tauri::command]
pub(crate) async fn library_sync_check_access(
    library: State<'_, Library>,
    expected_invite: String,
) -> Result<(), Failure> {
    let database = library.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = connect(&database).map_err(|error| storage_error(error.into()))?;
        let invite = read_status(&db)
            .map_err(storage_error)?
            .config
            .and_then(|config| config.invite)
            .filter(|invite| *invite == expected_invite)
            .ok_or(INVALID_INVITE)?;
        let profile = device_auth(&database).map_err(storage_error)?;
        let server = Server::new(Invite::parse(&invite).map_err(|_| INVALID_INVITE)?, profile)
            .map_err(server_error)?;
        check_access(&server)
    })
    .await
    .map_err(|_| STORAGE)?
}

#[tauri::command]
pub(crate) async fn library_sync_discover() -> Result<Vec<discovery::Peer>, String> {
    tauri::async_runtime::spawn_blocking(discovery::scan)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("Поиск устройств недоступен. {error}"))
}

#[tauri::command]
pub(crate) async fn library_sync_status(library: State<'_, Library>) -> Result<Status, Failure> {
    let database = library.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        read_status(&connect(&database).map_err(|error| storage_error(error.into()))?)
            .map_err(storage_error)
    })
    .await
    .map_err(|_| STORAGE)?
}

#[tauri::command]
pub(crate) async fn library_sync_configure(
    library: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    config: Option<Selection>,
) -> Result<Status, Failure> {
    let guard = Arc::clone(&library.sync_job)
        .try_lock_owned()
        .map_err(|_| BUSY)?;
    let database = library.database.clone();
    let roots = configured_roots(&folders)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let db = connect(&database).map_err(|error| storage_error(error.into()))?;
        let config = if let Some(selection) = config {
            let invite = selection
                .invite
                .as_deref()
                .map(str::trim)
                .filter(|code| !code.is_empty());
            let exchange_folder = if let Some(invite) = invite {
                Invite::parse(invite).map_err(|_| INVALID_INVITE)?;
                selection.folder.clone()
            } else {
                read_status(&db)
                    .map_err(storage_error)?
                    .config
                    .filter(|saved| saved.invite.is_none())
                    .map(|saved| saved.folder)
                    .ok_or(INVALID_INVITE)?
            };
            let config = Config {
                folder: exchange_folder,
                sources: Some(vec![selection.folder.clone()]),
                destination: selection.folder,
                automatic: selection.automatic,
                invite: invite.map(str::to_owned),
            };
            resolve_folders(&config, &roots)?;
            Some(config)
        } else {
            None
        };
        let raw = config
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| storage_error(error.into()))?;
        db.execute(
            "UPDATE sync_state SET config=?1, report=CASE
             WHEN json_extract(config,'$.folder')=json_extract(?1,'$.folder')
               AND json_extract(config,'$.destination')=json_extract(?1,'$.destination')
               AND json_extract(config,'$.sources') IS json_extract(?1,'$.sources')
               AND json_extract(config,'$.invite') IS json_extract(?1,'$.invite')
             THEN report ELSE NULL END",
            [raw],
        )
        .map_err(|error| storage_error(error.into()))?;
        read_status(&db).map_err(storage_error)
    })
    .await
    .map_err(|_| STORAGE)?
}

fn check_access(server: &Server) -> Result<(), Failure> {
    match server.check().map_err(server_error)? {
        Access::Allowed => Ok(()),
        Access::Pending => Err(ACCESS_PENDING),
        Access::Blocked => Err(ACCESS_BLOCKED),
        Access::Upgrade => Err(SERVER_UPGRADE),
    }
}

fn run(
    database: &Path,
    configured: &[String],
    roots: &Mutex<Vec<PathBuf>>,
    catalog_writer: &tokio::sync::Mutex<()>,
    cancelled: &AtomicBool,
    mut report_progress: impl FnMut(&'static str, usize, Option<usize>),
) -> Result<Option<Report>, Failure> {
    let started = Instant::now();
    report_progress("preparing", 0, None);
    let mut db = connect(database).map_err(|error| storage_error(error.into()))?;
    let status = read_status(&db).map_err(storage_error)?;
    let Some(config) = status.config else {
        return Ok(None);
    };
    let device: String = db
        .query_row("SELECT device FROM sync_state", [], |row| row.get(0))
        .map_err(|error| storage_error(error.into()))?;
    let (folder, destination, sources) = resolve_folders(&config, configured)?;
    let network = config.invite.is_some();
    let exchange = match &config.invite {
        Some(code) => {
            let invite = Invite::parse(code).map_err(|_| INVALID_INVITE)?;
            let profile = load_device(database, device.clone()).map_err(storage_error)?;
            let server = Server::new(invite, profile).map_err(server_error)?;
            check_access(&server)?;
            Exchange::Server(server)
        }
        None => Exchange::directory(folder.clone()).map_err(folder_error)?,
    };
    let exchange_error = |error: io::Error| {
        if network {
            server_error(error)
        } else {
            folder_error(error)
        }
    };
    let mut report = Report::default();
    let peers = read_peers(&exchange, &device, &mut report).map_err(exchange_error)?;
    report.peers = peers.len();
    let mut tracks = media::read_local(&db, sources, &peers).map_err(storage_error)?;
    let previous = exchange
        .manifest(&device, MAX_MANIFEST_BYTES)
        .map_err(exchange_error)?;
    let previous_entries: HashSet<_> = if config.sources.is_some() || network {
        previous
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<Manifest>(bytes).ok())
            .filter(|manifest| manifest.valid(&device))
            .map(|manifest| {
                manifest
                    .playlists
                    .into_iter()
                    .map(playlist::Register::into_key)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    let peer_tracks: HashSet<_> = peers
        .iter()
        .flat_map(|peer| peer.media.keys().map(String::as_str))
        .collect();
    let exchange_is_library_root = !network && configured.contains(&config.folder);
    let retired = media::retired(&db).map_err(|error| storage_error(error.into()))?;
    let (outgoing, incoming) = media::exchange(
        &mut tracks,
        &peers,
        media::Transfer {
            roots,
            destination: &destination,
            exchange: &exchange,
            exchange_is_library_root,
            retired: &retired,
            cancelled,
        },
        &mut report,
        &mut report_progress,
    );

    if cancelled.load(Ordering::Relaxed) {
        return Err(CANCELLED);
    }
    report_progress("applying", 0, None);
    let (manifest, changed) = {
        let _writer = catalog_writer.blocking_lock();
        if cancelled.load(Ordering::Relaxed) {
            return Err(CANCELLED);
        }
        {
            let registered = roots.lock().map_err(|_| STORAGE)?;
            if !registered.contains(&destination) {
                return Err(DESTINATION);
            }
            if exchange_is_library_root && !registered.contains(&folder) {
                return Err(EXCHANGE_FOLDER);
            }
        }
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error(error.into()))?;
        let changed = (|| -> LibraryResult<_> {
            let (media_changed, imported) = media::apply(&tx, &tracks, &incoming, &config)?;
            report.imported = imported;
            let (mut metadata_edits, metadata_changed) = metadata::merge(&tx, &device, &peers)?;
            if media_changed || metadata_changed {
                versions::rebuild(&tx)?;
            }
            let (mut playlists, playlists_changed) = playlist::merge(&tx, &device, &peers)?;
            report.playlists = playlists_changed;
            for peer in &peers {
                counter_merge(&tx, &peer.device, &peer.counts)?;
            }
            let (mut counts, counts_changed) = project_counts(&tx, &device)?;
            if config.sources.is_some() || network {
                let in_exchange =
                    |digest: &str| outgoing.contains_key(digest) || peer_tracks.contains(digest);
                counts.retain(|digest, _| in_exchange(digest));
                metadata_edits.retain(|edit| in_exchange(edit.digest()));
                playlists.retain(|entry| {
                    entry.in_scope(&in_exchange)
                        || entry
                            .removal_key()
                            .is_some_and(|key| previous_entries.contains(&key))
                });
            }
            let changed =
                media_changed || metadata_changed || playlists_changed > 0 || counts_changed;
            if changed {
                tx.execute("UPDATE catalog_state SET revision=revision+1", [])?;
            }
            Ok((
                Manifest {
                    version: VERSION,
                    device,
                    media: outgoing,
                    counts,
                    playlists,
                    metadata_edits,
                },
                changed,
            ))
        })()
        .map_err(storage_error)?;
        tx.commit().map_err(|error| storage_error(error.into()))?;
        changed
    };

    report_progress("publishing", 0, None);
    match publish(&exchange, &manifest, previous.as_deref()) {
        Ok(bytes) => report.manifest_bytes = bytes,
        Err(error) => {
            eprintln!("Library sync (publish): {error}");
            report.warn("publish");
        }
    }
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    report.transfer_ms = report.transfer_time.as_micros().div_ceil(1000) as u64;
    report.finished_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    report.catalog_changed = changed;
    if let Err(error) = serde_json::to_string(&report)
        .map_err(|error| storage_error(error.into()))
        .and_then(|json| {
            db.execute("UPDATE sync_state SET report=?1", [json])
                .map_err(|error| storage_error(error.into()))
        })
    {
        eprintln!("Library sync: {}", error.message);
    }
    Ok(Some(report))
}

#[tauri::command]
pub(crate) async fn library_sync_run(
    library: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    progress: Channel<Progress>,
) -> Result<Option<Report>, Failure> {
    let guard = Arc::clone(&library.sync_job)
        .try_lock_owned()
        .map_err(|_| BUSY)?;
    let database = library.database.clone();
    let writer = Arc::clone(&library.catalog_writer);
    let cancelled = Arc::clone(&library.sync_cancelled);
    cancelled.store(false, Ordering::Relaxed);
    let configured = configured_roots(&folders)?;
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let mut progress = ProgressReporter {
            channel: progress,
            phase: "",
            last: Instant::now(),
        };
        run(
            &database,
            &configured,
            &roots,
            &writer,
            &cancelled,
            |phase, current, total| progress.update(phase, current, total),
        )
    })
    .await
    .map_err(|_| STORAGE)?
}

#[tauri::command]
pub(crate) fn library_sync_cancel(library: State<'_, Library>) {
    library.sync_cancelled.store(true, Ordering::Relaxed);
}

pub(super) use media::apply_imported_metadata;

pub(super) use metadata::{extend as extend_metadata_edits, queue as queue_metadata_edits};
