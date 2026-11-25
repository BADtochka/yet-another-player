use std::{
    collections::{BTreeMap, HashMap, HashSet},
    future::Future,
    sync::Arc,
};

use tauri::{AppHandle, Emitter, Manager};
use tokio::task::JoinSet;

use super::{
    artist_profile, cache_artwork, find_release, normalized, release_from_refs, release_key,
    valid_text, ArtistProfile, Configuration, ReleaseSummary,
};
use crate::library::{connect, Library, LibraryResult, Tags};

struct LocalRelease {
    id: String,
    album: Option<String>,
    title: String,
    artists: Vec<String>,
    credit: String,
}

#[derive(Default)]
struct Targets {
    artists: BTreeMap<String, (String, Vec<String>)>,
    releases: Vec<LocalRelease>,
}

fn read_targets(library: &Library) -> LibraryResult<Targets> {
    let db = connect(library.database_path())?;
    let mut statement = db.prepare(
        "SELECT i.album_id, t.metadata FROM track_index i JOIN track t ON t.id=i.id
         WHERE EXISTS (SELECT 1 FROM source WHERE track_id=t.id)
           AND NOT EXISTS (SELECT 1 FROM track_variant WHERE variant_id=t.id)
         ORDER BY i.album_id, i.disc, i.track_number, i.name_key, t.path",
    )?;
    let mut targets = Targets::default();
    let mut releases = HashSet::new();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let album_id: String = row.get(0)?;
        let tags: Tags = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let title = tags.name.trim();
        for name in tags
            .artists
            .iter()
            .map(|artist| artist.trim())
            .filter(|name| valid_text(name))
        {
            let (_, samples) = targets
                .artists
                .entry(normalized(name))
                .or_insert_with(|| (name.to_owned(), Vec::new()));
            if samples.len() < 4
                && valid_text(title)
                && !samples.iter().any(|sample| sample == title)
            {
                samples.push(title.to_owned());
            }
        }
        let joined = tags.artists.join(", ");
        let credit = if joined.trim().is_empty() {
            tags.album_artist.as_deref().unwrap_or("").trim()
        } else {
            joined.trim()
        };
        let album = tags.album_name().filter(|album| valid_text(album));
        if valid_text(credit) && valid_text(title) && releases.insert(album_id.clone()) {
            targets.releases.push(LocalRelease {
                id: album_id,
                album: album.map(str::to_owned),
                title: title.to_owned(),
                credit: credit.to_owned(),
                artists: tags.artists,
            });
        }
    }
    Ok(targets)
}

#[derive(Clone, serde::Serialize)]
struct Progress {
    phase: &'static str,
    current: usize,
    total: usize,
}

async fn run_jobs<J, T, F>(
    app: &AppHandle,
    phase: &'static str,
    jobs: Vec<J>,
    current: &impl Fn() -> bool,
    task: impl Fn(J) -> F,
) -> Vec<T>
where
    T: Send + 'static,
    F: Future<Output = Option<T>> + Send + 'static,
{
    let total = jobs.len();
    if total == 0 || !current() {
        return Vec::new();
    }
    let report = |count| {
        let _ = app.emit(
            "library:release-progress",
            Progress {
                phase,
                current: count,
                total,
            },
        );
    };
    report(0);
    let mut jobs = jobs.into_iter();
    let mut pending = JoinSet::new();
    for job in jobs.by_ref().take(4) {
        pending.spawn(task(job));
    }
    let mut output = Vec::new();
    let mut count = 0;
    while let Some(result) = pending.join_next().await {
        if !current() {
            break;
        }
        if let Ok(Some(result)) = result {
            output.push(result);
        }
        count += 1;
        report(count);
        if let Some(job) = jobs.next() {
            pending.spawn(task(job));
        }
    }
    output
}

enum Job {
    Local(LocalRelease, Vec<ReleaseSummary>),
    Remote(ReleaseSummary),
}

pub(in crate::library) async fn warm_scan(
    app: &AppHandle,
    config: Configuration,
    token: Option<String>,
    current: impl Fn() -> bool,
) -> LibraryResult<()> {
    if !current() {
        return Ok(());
    }
    let targets = read_targets(&app.state::<Library>())?;
    let config = Arc::new(config);
    let token = token.map(Arc::<str>::from);
    let profiles = run_jobs(
        app,
        "artists",
        targets.artists.into_iter().collect(),
        &current,
        |(id, (name, samples))| {
            let app = app.clone();
            let config = Arc::clone(&config);
            let token = token.clone();
            async move {
                match artist_profile(
                    &app,
                    &app.state::<Library>(),
                    &name,
                    &samples,
                    token.as_deref(),
                    &config,
                    false,
                )
                .await
                {
                    Ok(Some(profile)) => Some((id, profile)),
                    Ok(None) => None,
                    Err(error) => {
                        eprintln!("Artist catalog: {error}");
                        None
                    }
                }
            }
        },
    )
    .await
    .into_iter()
    .collect::<HashMap<String, ArtistProfile>>();
    if !current() {
        return Ok(());
    }
    let _ = app.emit("library:release-catalog", ());
    let mut jobs = Vec::new();
    for release in targets.releases {
        let mut refs = HashSet::new();
        let key = release_key(release.album.as_deref().unwrap_or(&release.title));
        let known = release
            .artists
            .iter()
            .filter_map(|artist| profiles.get(&normalized(artist)))
            .flat_map(|profile| &profile.releases)
            .filter(|item| release_key(&item.title) == key && refs.insert(&item.reference))
            .cloned()
            .collect();
        jobs.push(Job::Local(release, known));
    }
    let mut referenced = HashSet::new();
    for profile in profiles.into_values() {
        jobs.extend(
            profile
                .releases
                .into_iter()
                .filter(|item| referenced.insert(item.reference.clone()))
                .map(Job::Remote),
        );
    }
    let artwork = run_jobs(app, "releases", jobs, &current, |job| {
        let app = app.clone();
        let config = Arc::clone(&config);
        async move {
            let library = app.state::<Library>();
            let (album_id, result) = match job {
                Job::Local(release, known) => (
                    Some(release.id),
                    find_release(
                        &app,
                        &library,
                        &release.credit,
                        &release.title,
                        release.album.as_deref(),
                        &config,
                        &known,
                    )
                    .await,
                ),
                Job::Remote(release) => (
                    None,
                    release_from_refs(
                        &app,
                        &library,
                        &config,
                        &release.reference,
                        &release.alternates,
                    )
                    .await,
                ),
            };
            match result {
                Ok(Some(release)) => {
                    let id = album_id?;
                    let url = release.artwork?;
                    cache_artwork(library.database_path(), &url)
                        .await
                        .then_some((id, url))
                }
                Ok(None) => None,
                Err(error) => {
                    eprintln!("Release catalog: {error}");
                    None
                }
            }
        }
    })
    .await;
    if current() {
        if !artwork.is_empty() {
            let library = app.state::<Library>();
            let _writer = library.lock_catalog_writer().await;
            if !current() {
                return Ok(());
            }
            let mut db = connect(library.database_path())?;
            let tx = db.transaction()?;
            for (id, url) in artwork {
                tx.execute(
                    "INSERT OR REPLACE INTO release_artwork (album_id, url)
                     SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM resolver_artwork WHERE url=?2)",
                    rusqlite::params![id, url],
                )?;
            }
            tx.commit()?;
            let _ = app.emit("library:metadata", ());
        }
        let _ = app.emit("library:release-catalog", ());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preloads_each_indexed_album_and_each_credited_artist_once() {
        let temp = tempfile::tempdir().unwrap();
        let library = Library::new(temp.path().join("library.sqlite3")).unwrap();
        let mut db = connect(library.database_path()).unwrap();
        let tx = db.transaction().unwrap();
        for (id, title, artists) in [
            (1, "Первый", vec!["VILLIAN", "Aarne"]),
            (2, "Второй", vec!["VILLIAN"]),
            (3, "Удалён", vec!["Another"]),
        ] {
            let tags = Tags {
                name: title.into(),
                artists: artists.into_iter().map(str::to_owned).collect(),
                album: Some("Красивое Зло".into()),
                album_artist: Some("VILLIAN".into()),
                ..Default::default()
            };
            let path = format!("/music/{id}.flac");
            tx.execute(
                "INSERT INTO track (id,path,stamp,size,metadata) VALUES (?1,?2,'',0,?3)",
                rusqlite::params![id, path, serde_json::to_string(&tags).unwrap()],
            )
            .unwrap();
            crate::library::queries::index(&tx, id, &path, &tags).unwrap();
            if id != 3 {
                tx.execute("INSERT INTO source VALUES ('/music',?1)", [id])
                    .unwrap();
            }
        }
        tx.commit().unwrap();
        let targets = read_targets(&library).unwrap();
        assert_eq!(targets.releases.len(), 1);
        assert_eq!(targets.releases[0].album.as_deref(), Some("Красивое Зло"));
        assert_eq!(targets.artists.len(), 2);
        assert_eq!(targets.artists["villian"].1.len(), 2);
        assert_eq!(targets.artists["aarne"].1, ["Первый"]);
    }
}
