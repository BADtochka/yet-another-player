use super::*;
use crate::{
    external_tools::{enabled::tool_enabled, install::Tool},
    genius,
};

pub(super) struct Sources {
    pub(super) order: Vec<MetadataSource>,
    ommr: LibraryResult<Option<Resolver>>,
    pub(super) token: Option<String>,
    client: Client,
}

impl Sources {
    pub(super) async fn configured(app: &AppHandle, config: Configuration) -> LibraryResult<Self> {
        use MetadataSource::*;
        let primary = config.primary;
        let ommr = if tool_enabled(app, Tool::Ommr) && primary != Off {
            configured_resolver(app, config).await
        } else {
            Ok(None)
        };
        let token = if genius::tool_enabled(app) && primary != Off {
            genius::access_token(app).await.unwrap_or_else(|error| {
                eprintln!("Genius credential store: {error:?}");
                None
            })
        } else {
            None
        };
        let mut order = Vec::new();
        for source in [primary, Ommr, Genius, MusicBrainz, Itunes] {
            let available = match source {
                Ommr => !matches!(&ommr, Ok(None)),
                Genius => token.is_some(),
                Itunes | MusicBrainz => true,
                Auto | Off => false,
            };
            if primary != Off && available && !order.contains(&source) {
                order.push(source);
            }
        }
        let client = Client::builder()
            .user_agent(concat!(
                "yet-another-player/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/BADtochka/yet-another-player)"
            ))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(12))
            .build()?;
        Ok(Self {
            order,
            ommr,
            token,
            client,
        })
    }

    pub(super) fn enabled(&self, app: &AppHandle, source: MetadataSource) -> bool {
        match source {
            MetadataSource::Ommr => tool_enabled(app, Tool::Ommr),
            MetadataSource::Genius => genius::tool_enabled(app),
            _ => true,
        }
    }

    pub(super) fn supports(&self, source: MetadataSource, input: &Lookup) -> bool {
        if source == MetadataSource::Ommr && input.reference.is_some() {
            return true;
        }
        let valid = !input.title.is_empty()
            && [&input.title, &input.artist, &input.album]
                .iter()
                .all(|field| field.len() <= 1024 && !field.contains(char::is_control));
        valid && (!input.artist.is_empty() || source == MetadataSource::Ommr)
    }

    pub(super) fn cache_key(
        &self,
        source: MetadataSource,
        input: &Lookup,
    ) -> LibraryResult<String> {
        if source == MetadataSource::Genius {
            // Retry misses from the previous Genius title and artist matching rules.
            return Ok(serde_json::to_string(&(
                "metadata",
                source,
                2,
                &input.artists,
                input,
            ))?);
        }
        if source == MetadataSource::Itunes || source == MetadataSource::MusicBrainz {
            return Ok(serde_json::to_string(&("metadata", source, 2, input))?);
        }
        let ommr = self.ommr.as_ref().ok().and_then(Option::as_ref);
        let scope = if source == MetadataSource::Ommr {
            ommr.map(|resolver| (&resolver.cache_scope, &resolver.sources))
        } else {
            None
        };
        Ok(serde_json::to_string(&("metadata", source, scope, input))?)
    }

    pub(super) async fn resolve(
        &self,
        app: &AppHandle,
        source: MetadataSource,
        input: &Lookup,
        next: &mut Option<Instant>,
        current: &(impl Fn() -> bool + Sync),
    ) -> LibraryResult<Option<Track>> {
        let available = || current() && self.enabled(app, source);
        match source {
            MetadataSource::Ommr => {
                let Some(resolver) = self.ommr.as_ref().map_err(|err| err.to_string())? else {
                    return Ok(None);
                };
                let mut track = resolver.resolve(input, next, &available).await?;
                if let Some(track) = &mut track {
                    if let Some(data) = &track.provider_data {
                        track.catalogs.extend(
                            data.other
                                .iter()
                                .filter(|(_, record)| !record.is_null())
                                .map(|(name, _)| name.clone()),
                        );
                        if data.deezer.is_some() {
                            track.catalogs.push("deezer".into());
                        }
                    }
                }
                Ok(track)
            }
            MetadataSource::Genius => {
                wait(next, &available, REQUEST_INTERVAL).await?;
                let Some(token) = &self.token else {
                    return Ok(None);
                };
                let song = genius::metadata(token, &input.title, &input.artists)
                    .await
                    .map_err(|error| format!("Genius: {error:?}"))?;
                Ok(song.map(genius_track))
            }
            MetadataSource::Itunes => {
                wait(next, &available, Duration::from_millis(3100)).await?;
                let term = format!("{} {}", input.artist, input.title);
                let response: ItunesResults = self
                    .read(
                        self.client
                            .get("https://itunes.apple.com/search")
                            .query(&[("term", term.as_str()), ("entity", "song"), ("limit", "20")])
                            .send()
                            .await?,
                    )
                    .await?;
                let candidate = response
                    .results
                    .into_iter()
                    .map(|song| (song.collection_id, Track::from(song)))
                    .filter(|(_, track)| matches_identity(track, input))
                    .rev()
                    .max_by_key(|(_, track)| matching_album(track, input));
                let Some((collection_id, mut track)) = candidate else {
                    return Ok(None);
                };
                if let (Some(id), Some(album)) = (collection_id, &mut track.album) {
                    // The song search can report a different date than the collection lookup.
                    track.release_date.clear();
                    wait(next, &available, Duration::from_millis(3100)).await?;
                    let lookup = async {
                        let id_query = id.to_string();
                        let response: ItunesReleaseResults = self
                            .read(
                                self.client
                                    .get("https://itunes.apple.com/lookup")
                                    .query(&[("id", id_query.as_str())])
                                    .send()
                                    .await?,
                            )
                            .await?;
                        Ok::<_, super::super::LibraryError>(response.results.into_iter().find(
                            |release| {
                                release.wrapper_type == "collection" && release.collection_id == id
                            },
                        ))
                    }
                    .await;
                    match lookup {
                        Ok(Some(release)) => {
                            album.release_date = release.release_date;
                            album.verified_date = true;
                        }
                        Ok(None) => {}
                        Err(error) => eprintln!("iTunes release {id}: {error}"),
                    }
                }
                Ok(Some(track))
            }
            MetadataSource::MusicBrainz => {
                wait(next, &available, REQUEST_INTERVAL).await?;
                let quote = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
                let query = format!(
                    "recording:\"{}\" AND artist:\"{}\"",
                    quote(&input.title),
                    quote(&input.artist)
                );
                let response: BrainzResults = self
                    .read(
                        self.client
                            .get("https://musicbrainz.org/ws/2/recording/")
                            .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "12")])
                            .send()
                            .await?,
                    )
                    .await?;
                Ok(response
                    .recordings
                    .into_iter()
                    .map(|recording| recording.track(input))
                    .filter(|track| matches_identity(track, input))
                    .rev()
                    .max_by_key(|track| matching_album(track, input)))
            }
            _ => Ok(None),
        }
    }

    async fn read<T: DeserializeOwned>(&self, response: reqwest::Response) -> LibraryResult<T> {
        let mut response = response.error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("Metadata response too large".into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&body)?)
    }
}

async fn wait(
    next: &mut Option<Instant>,
    available: &(impl Fn() -> bool + Sync),
    interval: Duration,
) -> LibraryResult<()> {
    while let Some(time) = next.filter(|time| *time > Instant::now()) {
        if !available() {
            return Err("Metadata search cancelled".into());
        }
        sleep_until(time.min(Instant::now() + Duration::from_secs(1))).await;
    }
    if !available() {
        return Err("Metadata search cancelled".into());
    }
    *next = Some(Instant::now() + interval);
    Ok(())
}

fn matches_identity(track: &Track, input: &Lookup) -> bool {
    let normalization = |text: &str| {
        text.chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    normalization(&track.title) == normalization(&input.title)
        && track.artists.as_ref().is_some_and(|artists| {
            artists
                .iter()
                .any(|artist| normalization(&artist.name) == normalization(&input.artist))
        })
        && (input.duration.is_none()
            || track.duration_ms.is_none()
            || track.matches_duration(input.duration))
}

fn matching_album(track: &Track, input: &Lookup) -> bool {
    track
        .album
        .as_ref()
        .is_some_and(|album| release_name(&album.title) == release_name(&input.album))
}

fn genius_track(song: genius::MetadataSong) -> Track {
    let album_artist = song
        .album
        .as_ref()
        .and_then(|album| album.artist.as_ref())
        .map(|artist| artist.name.clone());
    Track {
        title: song.title,
        artists: Some(vec![Artist {
            name: song.artist_names,
        }]),
        album: song.album.map(|album| Album {
            title: album.name,
            ..Album::default()
        }),
        release_date: song.release_date.unwrap_or_default(),
        album_artist,
        ..Track::default()
    }
}

#[derive(Deserialize)]
struct ItunesResults {
    results: Vec<ItunesTrack>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ItunesTrack {
    collection_id: Option<u64>,
    track_name: String,
    artist_name: String,
    collection_name: String,
    release_date: String,
    track_time_millis: Option<u64>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    primary_genre_name: String,
}

impl From<ItunesTrack> for Track {
    fn from(song: ItunesTrack) -> Self {
        Self {
            title: song.track_name,
            artists: Some(vec![Artist {
                name: song.artist_name,
            }]),
            album: Some(Album {
                title: song.collection_name,
                ..Album::default()
            }),
            release_date: song
                .release_date
                .split('T')
                .next()
                .unwrap_or_default()
                .into(),
            duration_ms: song.track_time_millis,
            track_number: song.track_number,
            disc_number: song.disc_number,
            genres: Some(vec![song.primary_genre_name]),
            ..Self::default()
        }
    }
}

#[derive(Deserialize)]
struct ItunesReleaseResults {
    results: Vec<ItunesRelease>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesRelease {
    collection_id: u64,
    wrapper_type: String,
    #[serde(default)]
    release_date: String,
}

#[derive(Deserialize)]
struct BrainzResults {
    recordings: Vec<Recording>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct Recording {
    title: String,
    artist_credit: Vec<Artist>,
    length: Option<u64>,
    first_release_date: String,
    releases: Vec<Release>,
    tags: Vec<BrainzTag>,
    isrcs: Vec<String>,
}

#[derive(Deserialize)]
struct Release {
    title: String,
    date: Option<String>,
    status: Option<String>,
    #[serde(rename = "release-group")]
    group: Option<BrainzReleaseGroup>,
}

#[derive(Deserialize)]
struct BrainzReleaseGroup {
    #[serde(rename = "primary-type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct BrainzTag {
    name: String,
    count: i32,
}

impl Recording {
    fn track(self, input: &Lookup) -> Track {
        let release = self
            .releases
            .into_iter()
            .filter(|release| {
                (release.status.as_deref() == Some("Official") && input.album.is_empty())
                    || release_name(&release.title) == release_name(&input.album)
            })
            .min_by_key(|release| release.date.clone().unwrap_or_else(|| "9999".into()));
        Track {
            title: self.title,
            artists: Some(self.artist_credit),
            album: release.map(|release| Album {
                kind: release.group.and_then(|group| group.kind),
                title: release.title,
                release_date: release.date.unwrap_or_default(),
                ..Album::default()
            }),
            duration_ms: self.length,
            release_date: self.first_release_date,
            genres: Some(
                self.tags
                    .into_iter()
                    .filter(|tag| tag.count > 0)
                    .take(5)
                    .map(|tag| tag.name)
                    .collect(),
            ),
            isrc: self.isrcs.into_iter().next().unwrap_or_default(),
            ..Track::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup() -> Lookup {
        Lookup {
            title: "Harder, Better, Faster, Stronger".into(),
            artist: "Daft Punk".into(),
            artists: vec!["Daft Punk".into()],
            album: "Discovery".into(),
            reference: None,
            duration: Some(224.0),
        }
    }

    #[test]
    fn itunes_matches_punctuation_variants_without_matching_remixes() {
        let response: ItunesResults = serde_json::from_str(r#"{"results":[
            {"trackName":"Harder Better Faster Stronger","artistName":"Daft Punk","collectionName":"Discovery",
             "releaseDate":"2001-02-26T08:00:00Z","trackTimeMillis":224000,"primaryGenreName":"Electronic"},
            {"trackName":"Harder Better Faster Stronger (Remix)","artistName":"Daft Punk","trackTimeMillis":224000}
        ]}"#).unwrap();
        let mut tracks = response.results.into_iter().map(Track::from);
        let mut tags = Tags::default();
        let song = tracks.next().unwrap();
        assert!(matches_identity(&song, &lookup()));
        assert!(!matches_identity(&tracks.next().unwrap(), &lookup()));
        song.fill(&mut tags, &mut DateOrigin::Track);
        assert_eq!(tags.release_date.as_deref(), Some("2001-02-26"));
        assert_eq!(tags.genres, ["Electronic"]);
    }

    #[test]
    fn musicbrainz_keeps_the_date_of_the_matching_release() {
        let result: BrainzResults = serde_json::from_str(
            r#"{"recordings":[{
            "title":"Harder, Better, Faster, Stronger", "length":224000,
            "artist-credit":[{"name":"Daft Punk"}], "first-release-date":"2001",
            "releases":[{"title":"Discovery","date":"2001-03-12","status":"Official"},
                        {"title":"Compilation","date":"2000-01-01","status":"Official"}],
            "tags":[{"name":"electronic","count":3},{"name":"unpopular","count":0}],
            "isrcs":["GBDUW0000065"]
        }]}"#,
        )
        .unwrap();
        let song = result
            .recordings
            .into_iter()
            .next()
            .unwrap()
            .track(&lookup());
        let mut tags = Tags::default();
        assert!(matches_identity(&song, &lookup()));
        song.fill(&mut tags, &mut DateOrigin::Track);
        assert_eq!(tags.release_date.as_deref(), Some("2001-03-12"));
        assert_eq!(tags.genres, ["electronic"]);
        assert_eq!(tags.isrc.as_deref(), Some("GBDUW0000065"));
    }
}
