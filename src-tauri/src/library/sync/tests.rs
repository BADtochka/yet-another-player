use std::sync::atomic::AtomicBool;

use super::*;
use crate::library::{
    playlists::{change_playlists, Change},
    queries::{read_collection_tracks, CollectionKind},
    read_catalog, scan, ReadWarnings, Tags,
};

struct Device {
    library: Library,
    root: String,
    _directory: tempfile::TempDir,
}

impl Device {
    fn new(shared: &Path) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap().join("Music");
        fs::create_dir(&root).unwrap();
        let library = Library::new(directory.path().join("data/library.sqlite3")).unwrap();
        let root = root.to_str().unwrap().to_owned();
        let config = Config {
            folder: shared.to_str().unwrap().into(),
            destination: root.clone(),
            sources: None,
            automatic: false,
            invite: None,
        };
        connect(&library.database)
            .unwrap()
            .execute(
                "UPDATE sync_state SET config=?1",
                [serde_json::to_string(&config).unwrap()],
            )
            .unwrap();
        Self {
            library,
            root,
            _directory: directory,
        }
    }

    fn db(&self) -> Connection {
        connect(&self.library.database).unwrap()
    }

    fn file(&self, name: &str, marker: u8) -> PathBuf {
        let path = Path::new(&self.root).join(format!("{name}.opus"));
        let mut content = include_bytes!("../fixtures/one-sample.opus").to_vec();
        content.push(marker);
        fs::write(&path, content).unwrap();
        path
    }

    fn scan(&self) {
        self.scan_roots(std::slice::from_ref(&self.root));
    }

    fn scan_roots(&self, roots: &[String]) {
        scan(
            &mut self.db(),
            roots,
            &AtomicBool::new(false),
            &mut ReadWarnings::default(),
            |_| {},
        )
        .unwrap();
    }

    fn configure(&self, folder: &Path, sources: Vec<String>) {
        let config = Config {
            folder: folder.to_str().unwrap().into(),
            destination: self.root.clone(),
            sources: Some(sources),
            automatic: false,
            invite: None,
        };
        self.db()
            .execute(
                "UPDATE sync_state SET config=?1",
                [serde_json::to_string(&config).unwrap()],
            )
            .unwrap();
    }

    fn published(&self, folder: &Path) -> Manifest {
        let device: String = self
            .db()
            .query_row("SELECT device FROM sync_state", [], |row| row.get(0))
            .unwrap();
        let path = folder
            .join(EXCHANGE_DIR)
            .join("devices")
            .join(format!("{device}.json"));
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn id(&self, path: &Path) -> i64 {
        self.db()
            .query_row(
                "SELECT id FROM track WHERE path=?1",
                [path.to_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn run(&self) -> Report {
        self.sync_roots(std::slice::from_ref(&self.root))
    }

    fn sync_roots(&self, configured: &[String]) -> Report {
        let roots = Mutex::new(
            configured
                .iter()
                .map(|path| fs::canonicalize(path).unwrap())
                .collect(),
        );
        run(
            &self.library.database,
            configured,
            &roots,
            &self.library.catalog_writer,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
        .unwrap_or_else(|error| panic!("{}: {}", error.code, error.message))
        .unwrap()
    }

    fn playlist(&self, name: &str) -> i64 {
        self.db()
            .query_row("SELECT id FROM playlist WHERE name=?1", [name], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn change(&self, change: Change) {
        change_playlists(&mut self.db(), change).unwrap();
    }

    fn members(&self, name: &str) -> Vec<String> {
        read_collection_tracks(
            &self.db(),
            CollectionKind::Playlist,
            &self.playlist(name).to_string(),
        )
        .unwrap()
        .into_iter()
        .map(|track| track.tags.name)
        .collect()
    }

    fn set_tags(&self, path: &Path, name: &str, artists: &[&str]) {
        let mut db = self.db();
        let tx = db.transaction().unwrap();
        let id = self.id(path);
        let raw: String = tx
            .query_row("SELECT metadata FROM track WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        let mut tags: Tags = serde_json::from_str(&raw).unwrap();
        tags.name = name.into();
        tags.artists = artists.iter().map(|name| (*name).into()).collect();
        tx.execute(
            "UPDATE track SET metadata=?2 WHERE id=?1",
            params![id, serde_json::to_string(&tags).unwrap()],
        )
        .unwrap();
        crate::library::queries::index(&tx, id, path.to_str().unwrap(), &tags).unwrap();
        tx.commit().unwrap();
    }
}

#[test]
fn imports_files_and_credits_once_and_sums_completed_plays_per_device() {
    let shared = tempfile::tempdir().unwrap();
    let a = Device::new(shared.path());
    let b = Device::new(shared.path());
    let song_a = a.file("first", 1);
    let duplicate = a.file("copy", 1);
    let featured = a.file("featured", 2);
    let song_b = b.file("another-name", 1);
    a.scan();
    b.scan();
    a.set_tags(&featured, "Дуэт", &["Север", "Юг"]);
    a.library.record_play(song_a.to_str().unwrap()).unwrap();
    a.library.record_play(duplicate.to_str().unwrap()).unwrap();
    b.library.record_play(song_b.to_str().unwrap()).unwrap();
    a.change(Change::Create {
        name: "В дороге".into(),
        mix_enabled: true,
    });
    let id = a.playlist("В дороге");
    for path in [&song_a, &duplicate, &featured] {
        a.change(Change::Add {
            id,
            track_id: a.id(path),
        });
    }

    assert_eq!(a.run().duplicates, 1);
    let received = b.run();
    assert_eq!(received.imported, 1);
    assert!(received.bytes_received > 0);
    assert!(received.warnings.is_empty());
    a.run();
    b.run();
    let steady = b.run();
    assert_eq!(
        (
            steady.bytes_hashed,
            steady.bytes_sent,
            steady.bytes_received,
            steady.manifest_bytes
        ),
        (0, 0, 0, 0)
    );
    let tracks = read_catalog(&b.db()).unwrap().tracks;
    assert_eq!(tracks.len(), 2);
    assert_eq!(
        tracks
            .iter()
            .find(|track| track.path == song_b.to_str().unwrap())
            .unwrap()
            .play_count,
        3
    );
    for artist in ["север", "юг"] {
        assert_eq!(
            read_collection_tracks(&b.db(), CollectionKind::Artist, artist).unwrap()[0]
                .tags
                .name,
            "Дуэт"
        );
    }
    assert_eq!(b.members("В дороге").len(), 2);
    b.scan();
    assert!(b.members("В дороге").contains(&"Дуэт".to_owned()));
    b.library.record_play(song_b.to_str().unwrap()).unwrap();
    b.run();
    a.run();
    let tracks = read_catalog(&a.db()).unwrap().tracks;
    assert!(tracks
        .iter()
        .filter(|track| track.path != featured.to_str().unwrap())
        .all(|track| track.play_count == 4));
}

#[test]
fn merges_concurrent_playlist_changes_and_keeps_the_deletion() {
    let shared = tempfile::tempdir().unwrap();
    let a = Device::new(shared.path());
    let b = Device::new(shared.path());
    let paths = [a.file("first", 1), a.file("second", 2), a.file("third", 3)];
    a.scan();
    for (index, path) in paths.iter().enumerate() {
        a.set_tags(path, &format!("Track {index}"), &["Artist"]);
    }
    a.change(Change::Create {
        name: "Playlist".into(),
        mix_enabled: false,
    });
    let a_id = a.playlist("Playlist");
    a.change(Change::Add {
        id: a_id,
        track_id: a.id(&paths[0]),
    });
    a.run();
    b.run();
    let b_id = b.playlist("Playlist");
    let b_track = read_catalog(&b.db())
        .unwrap()
        .tracks
        .into_iter()
        .find(|track| track.tags.name == "Track 2")
        .unwrap()
        .id;

    a.change(Change::Add {
        id: a_id,
        track_id: a.id(&paths[1]),
    });
    b.change(Change::Rename {
        id: b_id,
        name: "New name".into(),
    });
    b.change(Change::Add {
        id: b_id,
        track_id: b_track,
    });
    a.run();
    b.run();
    a.run();
    let order = a.members("New name");
    assert_eq!(order.len(), 3);
    assert_eq!(order[0], "Track 0");
    assert_eq!(order, b.members("New name"));
    a.change(Change::Remove {
        id: a_id,
        track_id: a.id(&paths[1]),
    });
    a.run();
    b.run();
    assert_eq!(b.members("New name"), vec!["Track 0", "Track 2"]);

    b.change(Change::Delete { id: b_id });
    b.run();
    a.run();
    a.run();
    b.run();
    for device in [&a, &b] {
        assert_eq!(
            device
                .db()
                .query_row("SELECT count(*) FROM playlist", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn retries_missing_and_corrupt_blobs_without_losing_playlist_membership() {
    let shared = tempfile::tempdir().unwrap();
    let a = Device::new(shared.path());
    let b = Device::new(shared.path());
    let song = a.file("track", 3);
    a.scan();
    a.set_tags(&song, "Трек", &["Артист"]);
    a.change(Change::Create {
        name: "Playlist".into(),
        mix_enabled: true,
    });
    a.change(Change::Add {
        id: a.playlist("Playlist"),
        track_id: a.id(&song),
    });
    a.run();

    let dir = shared.path().join("YAP-Sync-v1/tracks");
    let blob = fs::read_dir(dir).unwrap().next().unwrap().unwrap().path();
    let bytes = fs::read(&blob).unwrap();
    fs::remove_file(&blob).unwrap();
    assert_eq!(b.run().warnings.get("receiveTrack"), Some(&1));
    assert!(b.members("Playlist").is_empty());
    fs::write(&blob, vec![0; bytes.len()]).unwrap();
    assert_eq!(b.run().warnings.get("receiveTrack"), Some(&1));
    assert!(read_catalog(&b.db()).unwrap().tracks.is_empty());

    assert_eq!(a.run().bytes_sent, bytes.len() as u64);
    assert_eq!(fs::read(&blob).unwrap(), bytes);
    assert_eq!(b.run().imported, 1);
    assert_eq!(b.members("Playlist"), vec!["Трек"]);

    let track = read_catalog(&b.db()).unwrap().tracks.remove(0);
    fs::remove_file(track.path).unwrap();
    b.db()
        .execute(
            "INSERT INTO track_trash (track_id,removed_at,directory,roots,stamp,size,metadata)
         SELECT id,unixepoch(),'','[]',stamp,size,metadata FROM track WHERE id=?1",
            [track.id],
        )
        .unwrap();
    b.db()
        .execute("DELETE FROM source WHERE track_id=?1", [track.id])
        .unwrap();
    assert_eq!(b.run().imported, 0);
    assert!(b.members("Playlist").is_empty());
}

#[cfg(unix)]
#[test]
fn shared_directory_and_blobs_reject_symlinks() {
    let folder = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(folder.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("YAP-Sync-v1")).unwrap();
    assert!(child_dir(&root, "YAP-Sync-v1").is_err());
    let data = outside.path().join("data");
    fs::write(&data, b"outside").unwrap();
    let link = root.join("fake-blob");
    std::os::unix::fs::symlink(data, &link).unwrap();
    assert!(read_file(&link, MAX_MANIFEST_BYTES).is_err());
}

#[test]
fn merges_artist_and_title_edits_on_existing_copies() {
    let shared = tempfile::tempdir().unwrap();
    let a = Device::new(shared.path());
    let b = Device::new(shared.path());
    let first = a.file("first", 7);
    let second = b.file("second", 7);
    a.scan();
    b.scan();
    a.run();
    b.run();
    let edit = |device: &Device, track: &Path, fields: serde_json::Value| {
        let id = device.id(track);
        let fields = fields.as_object().unwrap();
        let mut db = device.db();
        let tx = db.transaction().unwrap();
        tx.execute(
            "INSERT INTO track_edits VALUES (?1,?2,0)",
            params![id, serde_json::to_string(fields).unwrap()],
        )
        .unwrap();
        metadata::queue(&tx, id, fields).unwrap();
        tx.commit().unwrap();
    };
    edit(
        &a,
        &first,
        serde_json::json!({"artists": ["Guest", "Lead"]}),
    );
    edit(&b, &second, serde_json::json!({"name": "Correct title"}));
    a.run();
    b.run();
    a.run();
    for device in [&a, &b] {
        let tracks = read_collection_tracks(&device.db(), CollectionKind::Artist, "guest").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].tags.name, "Correct title");
        assert_eq!(tracks[0].tags.artists, ["Guest", "Lead"]);
    }
}

#[test]
fn cancelled_transfer_does_not_commit_a_manifest_or_catalog_changes() {
    let shared = tempfile::tempdir().unwrap();
    let a = Device::new(shared.path());
    let b = Device::new(shared.path());
    a.file("first", 1);
    a.scan();
    a.run();
    let roots = Mutex::new(vec![fs::canonicalize(&b.root).unwrap()]);
    let cancelled = AtomicBool::new(false);
    let result = run(
        &b.library.database,
        std::slice::from_ref(&b.root),
        &roots,
        &b.library.catalog_writer,
        &cancelled,
        |phase, _, _| {
            if phase == "receiving" {
                cancelled.store(true, Ordering::Relaxed);
            }
        },
    );
    assert!(matches!(
        result,
        Err(Failure {
            code: "cancelled",
            ..
        })
    ));
    assert!(read_status(&b.db()).unwrap().report.is_none());
    assert!(read_catalog(&b.db()).unwrap().tracks.is_empty());
    assert_eq!(b.run().imported, 1);
}

#[test]
fn keeps_counts_for_the_original_content_after_a_file_is_replaced() {
    let shared = tempfile::tempdir().unwrap();
    let a = Device::new(shared.path());
    let b = Device::new(shared.path());
    let original = a.file("first", 1);
    let duplicate = a.file("second", 1);
    a.scan();
    for file in [&original, &duplicate] {
        a.library.record_play(file.to_str().unwrap()).unwrap();
    }
    a.run();
    b.run();

    a.library.record_play(original.to_str().unwrap()).unwrap();
    fs::write(
        &original,
        [
            include_bytes!("../fixtures/one-sample.opus").as_slice(),
            &[2, 3],
        ]
        .concat(),
    )
    .unwrap();
    a.scan();
    a.library.record_play(original.to_str().unwrap()).unwrap();
    a.run();
    b.run();
    a.library.record_play(duplicate.to_str().unwrap()).unwrap();
    a.run();
    b.run();
    let mut counts = read_catalog(&b.db())
        .unwrap()
        .tracks
        .into_iter()
        .map(|track| track.play_count)
        .collect::<Vec<_>>();
    counts.sort_unstable();
    assert_eq!(counts, [1, 4]);
}

#[test]
fn only_selected_library_folders_publish_audio_and_track_data() {
    let legacy_share = tempfile::tempdir().unwrap();
    let new_share = tempfile::tempdir().unwrap();
    let device = Device::new(legacy_share.path());
    let private = device.file("private", 1);
    let selected = fs::canonicalize(device._directory.path())
        .unwrap()
        .join("Selected");
    fs::create_dir(&selected).unwrap();
    let public = selected.join("public.opus");
    fs::write(
        &public,
        [
            include_bytes!("../fixtures/one-sample.opus").as_slice(),
            &[2],
        ]
        .concat(),
    )
    .unwrap();
    let selected_root = selected.to_str().unwrap().to_owned();
    let roots = [
        device.root.clone(),
        selected_root.clone(),
        fs::canonicalize(new_share.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    ];
    device.scan_roots(&roots);
    device.change(Change::Create {
        name: "Mixed".into(),
        mix_enabled: false,
    });
    for file in [&public, &private] {
        device.change(Change::Add {
            id: device.playlist("Mixed"),
            track_id: device.id(file),
        });
    }
    device
        .library
        .record_play(private.to_str().unwrap())
        .unwrap();
    metadata::queue(
        &device.db(),
        device.id(&private),
        serde_json::json!({"name": "Private edit"})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    device.sync_roots(&roots);
    let before = device.published(legacy_share.path());
    assert_eq!(before.media.len(), 2);
    assert_eq!(before.metadata_edits.len(), 1);
    assert_eq!(before.counts.len(), 1);

    device.configure(Path::new(&roots[2]), vec![selected_root]);
    device.sync_roots(&roots);
    let after = device.published(new_share.path());
    assert_eq!(after.media.len(), 1);
    assert!(after.metadata_edits.is_empty());
    assert!(after.counts.is_empty());
    let playlist = serde_json::to_value(after.playlists).unwrap();
    assert_eq!(
        playlist
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["value"]["type"] == "track")
            .count(),
        1
    );
    assert_eq!(device.members("Mixed").len(), 2);
}

#[test]
fn devices_can_use_a_library_root_for_exchange_without_importing_a_second_copy() {
    let shared = tempfile::tempdir().unwrap();
    let shared_root = fs::canonicalize(shared.path()).unwrap();
    let share = shared_root.to_str().unwrap().to_owned();
    let a = Device::new(&shared_root);
    let b = Device::new(&shared_root);
    let a_roots = [a.root.clone(), share.clone()];
    let b_roots = [b.root.clone(), share.clone()];
    let file = shared_root.join("already-shared.opus");
    fs::write(&file, include_bytes!("../fixtures/one-sample.opus")).unwrap();
    a.scan_roots(&a_roots);
    a.configure(&shared_root, vec![share.clone()]);
    b.configure(&shared_root, vec![b.root.clone()]);
    a.sync_roots(&a_roots);

    let report = b.sync_roots(&b_roots);
    assert_eq!(report.peers, 1);
    assert_eq!(report.imported, 1);
    assert_eq!(report.bytes_received, 0);
    assert_eq!(
        read_catalog(&b.db()).unwrap().tracks[0].path,
        file.to_str().unwrap()
    );
    assert!(!Path::new(&b.root).join("YAP Sync").exists());

    let protocol_file = shared_root
        .join(EXCHANGE_DIR)
        .join("tracks/dont-index.opus");
    fs::write(protocol_file, include_bytes!("../fixtures/one-sample.opus")).unwrap();
    b.scan_roots(&b_roots);
    assert_eq!(read_catalog(&b.db()).unwrap().tracks.len(), 1);
}

#[test]
#[ignore = "build the standalone Go server and provide YAP_SYNC_TEST_SERVER"]
fn two_catalogs_exchange_tracks_plays_and_playlists_over_tls() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Child, Command, Stdio},
    };

    struct Running(Child);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let storage = tempfile::tempdir().unwrap();
    let a = Device::new(storage.path());
    let b = Device::new(storage.path());
    let host = device_auth(a.library.database_path()).unwrap();
    let executable = std::env::var_os("YAP_SYNC_TEST_SERVER").expect("set YAP_SYNC_TEST_SERVER");
    let mut server = Running(
        Command::new(executable)
            .args(["--listen", "127.0.0.1:0", "--data"])
            .arg(storage.path())
            .arg("--managed")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    server
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&host).unwrap())
        .unwrap();
    let stdout = server.0.stdout.take().unwrap();
    let mut invite = String::new();
    BufReader::new(stdout).read_line(&mut invite).unwrap();
    let pin = Invite::parse(&invite).unwrap();
    assert!(pin.modern());
    let identity: serde_json::Value =
        serde_json::from_slice(&fs::read(storage.path().join("identity.json")).unwrap()).unwrap();
    let admin_key = identity["admin"].as_str().unwrap();
    let admin = reqwest::blocking::Client::builder()
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(pin.certificate().unwrap())
        .build()
        .unwrap();
    let guest = device_auth(b.library.database_path()).unwrap();
    let change_access = |status: &str| {
        admin
            .put(pin.url(&format!("/admin/devices/{}", guest.id)))
            .bearer_auth(admin_key)
            .body(serde_json::json!({ "status": status }).to_string())
            .send()
            .unwrap()
            .error_for_status()
            .unwrap();
    };
    let attempt = |device: &Device| {
        run(
            device.library.database_path(),
            std::slice::from_ref(&device.root),
            &Mutex::new(vec![fs::canonicalize(&device.root).unwrap()]),
            &device.library.catalog_writer,
            &AtomicBool::new(false),
            |_, _, _| {},
        )
    };

    for device in [&a, &b] {
        let config = Config {
            folder: device.root.clone(),
            destination: device.root.clone(),
            sources: None,
            automatic: false,
            invite: Some(invite.trim().into()),
        };
        device
            .db()
            .execute(
                "UPDATE sync_state SET config=?1",
                [serde_json::to_string(&config).unwrap()],
            )
            .unwrap();
    }
    let track = a.file("one", 1);
    let same_track = b.file("same-with-another-name", 1);
    let second = b.file("two", 2);
    a.scan();
    b.scan();
    for (device, file) in [(&a, &track), (&b, &same_track)] {
        device.library.record_play(file.to_str().unwrap()).unwrap();
    }
    b.set_tags(&second, "Second", &["Guest"]);
    a.change(Change::Create {
        name: "List".into(),
        mix_enabled: false,
    });
    a.change(Change::Add {
        id: a.playlist("List"),
        track_id: a.id(&track),
    });

    let first = a.run();
    assert_eq!(first.peers, 0);
    assert!(first.bytes_sent > 0 && first.manifest_bytes > 0);
    assert!(matches!(
        attempt(&b),
        Err(Failure {
            code: "pairingPending",
            ..
        })
    ));
    assert!(!Path::new(&b.root).join("YAP Sync").exists());
    let pending = admin
        .get(pin.url("/admin/devices"))
        .bearer_auth(admin_key)
        .send()
        .unwrap()
        .text()
        .unwrap();
    let pending: Vec<serde_json::Value> = serde_json::from_str(&pending).unwrap();
    assert!(pending
        .iter()
        .any(|device| { device["id"] == guest.id && device["status"] == "pending" }));
    change_access("allowed");
    let exchange = b.run();
    assert_eq!(exchange.peers, 1);
    assert_eq!(exchange.imported, 0);
    assert_eq!(b.members("List").len(), 1);
    let received = a.run();
    assert_eq!(received.imported, 1);
    assert!(received.bytes_received > 0 && received.warnings.is_empty());
    let tracks = read_catalog(&a.db()).unwrap().tracks;
    assert_eq!(
        tracks
            .iter()
            .find(|item| item.path == track.to_str().unwrap())
            .unwrap()
            .play_count,
        2
    );
    assert_eq!(
        read_collection_tracks(&a.db(), CollectionKind::Artist, "guest").unwrap()[0]
            .tags
            .name,
        "Second"
    );
    a.run();
    let steady = a.run();
    assert_eq!(
        (
            steady.bytes_hashed,
            steady.bytes_sent,
            steady.bytes_received,
            steady.manifest_bytes
        ),
        (0, 0, 0, 0)
    );
    change_access("blocked");
    assert!(matches!(
        attempt(&b),
        Err(Failure {
            code: "pairingBlocked",
            ..
        })
    ));
}

#[test]
#[ignore = "requires the standalone server and an active IPv4 LAN adapter"]
fn discovery_finds_the_server_on_the_primary_adapter() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    let ip = discovery::primary_ipv4().expect("the host needs an active private IPv4 adapter");
    let root = tempfile::tempdir().unwrap();
    let binary = std::env::var_os("YAP_SYNC_TEST_SERVER").expect("set YAP_SYNC_TEST_SERVER");
    let mut child = Command::new(binary)
        .args(["--listen", &format!("{ip}:0"), "--data"])
        .arg(root.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let check = (|| -> LibraryResult<()> {
        let stdout = child.stdout.take().ok_or("Missing server stdout")?;
        let mut code = String::new();
        BufReader::new(stdout).read_line(&mut code)?;
        let own = Invite::parse(&code)?;
        let results = serde_json::to_value(discovery::scan()?)?;
        if !results.as_array().is_some_and(|peers| {
            peers.iter().any(|peer| {
                peer["address"] == own.address.to_string() && peer["invite"] == code.trim()
            })
        }) {
            return Err("The primary adapter did not discover the test server".into());
        }
        Ok(())
    })();
    let _ = child.kill();
    let _ = child.wait();
    check.unwrap();
}
