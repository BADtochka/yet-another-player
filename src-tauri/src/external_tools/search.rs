use std::{collections::HashSet, path::Path, time::Duration};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use super::{installed_yt_dlp, run_yt_dlp, valid_youtube_id, youtube_url, YtDlpError};
use crate::library::queries::search_words;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(16);
const DETAILS_TIMEOUT: Duration = Duration::from_secs(7);
const OUTPUT_LIMIT: u64 = 2 * 1024 * 1024;
const METADATA: &str =
    "%(.{id,title,track,artists,artist,creator,uploader,channel,duration,thumbnail,thumbnails,description})j";

#[derive(Default, Deserialize)]
#[serde(default)]
struct Item {
    id: String,
    title: Option<String>,
    track: Option<String>,
    artists: Option<Vec<String>>,
    artist: Option<String>,
    creator: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    description: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    thumbnails: Option<Vec<Thumbnail>>,
}

#[derive(Deserialize)]
struct Thumbnail {
    url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Serialize)]
pub(crate) struct SearchResult {
    id: String,
    title: String,
    artist: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

fn thumbnail_url(text: &str) -> bool {
    Url::parse(text).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| {
                host == "i.ytimg.com"
                    || host.ends_with(".ytimg.com")
                    || host == "lh3.googleusercontent.com"
                    || host.ends_with(".ggpht.com")
            })
    })
}

impl Item {
    fn title(&self) -> Option<&str> {
        nonempty(self.track.as_deref()).or_else(|| nonempty(self.title.as_deref()))
    }

    fn credited_artist(&self) -> Option<String> {
        self.artists
            .as_deref()
            .map(|artists| {
                artists
                    .iter()
                    .map(String::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|artists| !artists.is_empty())
            .or_else(|| {
                nonempty(self.artist.as_deref())
                    .or_else(|| nonempty(self.creator.as_deref()))
                    .map(str::to_owned)
            })
    }

    fn result(&self) -> Option<SearchResult> {
        let artist = self.credited_artist().or_else(|| {
            let uploader =
                nonempty(self.uploader.as_deref()).or_else(|| nonempty(self.channel.as_deref()))?;
            Some(
                uploader
                    .strip_suffix(" - Topic")
                    .unwrap_or(uploader)
                    .to_owned(),
            )
        });
        let thumbnail = self
            .thumbnail
            .as_deref()
            .filter(|text| thumbnail_url(text))
            .or_else(|| {
                self.thumbnails
                    .as_deref()?
                    .iter()
                    .filter(|image| image.url.as_deref().is_some_and(thumbnail_url))
                    .max_by_key(|image| {
                        u64::from(image.width.unwrap_or(0)) * u64::from(image.height.unwrap_or(0))
                    })
                    .and_then(|image| image.url.as_deref())
            });
        Some(SearchResult {
            id: self.id.clone(),
            title: self.title()?.to_owned(),
            artist,
            duration: self
                .duration
                .filter(|duration| duration.is_finite() && *duration > 0.0),
            thumbnail: Some(
                thumbnail
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", self.id)),
            ),
        })
    }

    fn is_music_video(&self) -> bool {
        if self
            .duration
            .is_none_or(|duration| !(15.0..=1800.0).contains(&duration))
        {
            return false;
        }
        let channel = self
            .uploader
            .as_deref()
            .or(self.channel.as_deref())
            .unwrap_or("")
            .to_lowercase();
        let title = self.title().unwrap_or("").to_lowercase();
        let non_original = [
            "реакция",
            " смотрит ",
            "reaction",
            "review",
            "ai cover",
            "slowed",
            "sped up",
            "perfect loop",
        ];
        !non_original.iter().any(|marker| title.contains(marker))
            && (self.track.is_some()
                || self.credited_artist().is_some()
                || self
                    .description
                    .as_deref()
                    .is_some_and(|description| description.starts_with("Provided to YouTube by "))
                || channel.ends_with(" - topic")
                || channel.ends_with("vevo")
                || [
                    "official music video",
                    "official audio",
                    "official video",
                    "official lyric video",
                    "официальный клип",
                    "официальное видео",
                ]
                .iter()
                .any(|marker| title.contains(marker)))
    }
}

fn music_url(query: &str) -> Result<Url, YtDlpError> {
    let mut url =
        Url::parse("https://music.youtube.com/search").map_err(|_| YtDlpError::RequestFailed)?;
    url.query_pairs_mut().append_pair("q", query);
    url.set_fragment(Some("songs"));
    Ok(url)
}

fn direct_video(query: &str) -> Option<String> {
    let url = Url::parse(query).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let id = match url.host_str()? {
        "youtu.be" => url.path_segments()?.next()?.to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com"
            if url.path() == "/watch" =>
        {
            url.query_pairs()
                .find(|(key, _)| key == "v")?
                .1
                .into_owned()
        }
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            let mut segments = url.path_segments()?;
            if !matches!(segments.next()?, "shorts" | "live" | "embed") {
                return None;
            }
            segments.next()?.to_owned()
        }
        _ => return None,
    };
    valid_youtube_id(&id).then_some(id)
}

async fn run_search(
    path: &Path,
    target: &str,
    flat: bool,
    limit: &str,
) -> Result<Vec<Item>, YtDlpError> {
    let args = [
        "--ignore-config",
        "--no-plugin-dirs",
        if flat {
            "--flat-playlist"
        } else {
            "--no-flat-playlist"
        },
        "--skip-download",
        "--ignore-no-formats-error",
        "--no-warnings",
        "--socket-timeout",
        "5",
        "--retries",
        "0",
        "--extractor-retries",
        "0",
        "--playlist-end",
        limit,
        "--print",
        METADATA,
        "--",
        target,
    ];
    let timeout = if flat {
        SEARCH_TIMEOUT
    } else {
        DETAILS_TIMEOUT
    };
    let (status, stdout) = run_yt_dlp(path, &args, timeout, OUTPUT_LIMIT).await?;
    let results = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Item>(line).ok())
        .filter(|item| valid_youtube_id(&item.id) && item.title().is_some())
        .collect::<Vec<_>>();
    if status.success() || !results.is_empty() {
        Ok(results)
    } else {
        Err(YtDlpError::RequestFailed)
    }
}

fn rank(query: &str, songs: Vec<SearchResult>, videos: Vec<SearchResult>) -> Vec<SearchResult> {
    let query = query.to_lowercase();
    let words = search_words(&query);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for (result, song) in songs
        .into_iter()
        .map(|item| (item, true))
        .chain(videos.into_iter().map(|item| (item, false)))
    {
        let title = result.title.to_lowercase();
        let context = format!(
            "{title} {}",
            result.artist.as_deref().unwrap_or("").to_lowercase()
        );
        let matched = words.iter().filter(|word| context.contains(**word)).count();
        let full = !words.is_empty() && matched == words.len();
        if (song || full) && seen.insert(result.id.clone()) {
            candidates.push(((full, title == query, matched, song), result));
        }
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    candidates
        .into_iter()
        .take(12)
        .map(|(_, result)| result)
        .collect()
}

#[tauri::command]
pub(crate) async fn search_yt_dlp(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<SearchResult>, YtDlpError> {
    let query = query.trim();
    if query.chars().count() < 2 || query.len() > 2048 || query.contains(char::is_control) {
        return Err(YtDlpError::InvalidQuery);
    }
    let path = installed_yt_dlp(&app).await?;
    if let Some(id) = direct_video(query) {
        let url = youtube_url(&id)?;
        let items = run_search(&path.path, &url, false, "1").await?;
        return Ok(items
            .into_iter()
            .filter(|item| item.id == id)
            .filter_map(|item| item.result())
            .collect());
    }

    let music = music_url(query)?;
    let video = format!("ytsearch16:{query}");
    let (songs, videos) = tokio::join!(
        run_search(&path.path, music.as_str(), true, "12"),
        run_search(&path.path, &video, true, "16")
    );
    let (songs, videos) = match (songs, videos) {
        (Ok(songs), Ok(videos)) => (songs, videos),
        (Ok(songs), Err(_)) if !songs.is_empty() => (songs, Vec::new()),
        (Err(_), Ok(videos)) if videos.iter().any(Item::is_music_video) => (Vec::new(), videos),
        (Err(error), _) | (_, Err(error)) => return Err(error),
    };

    let mut songs: Vec<_> = songs.iter().filter_map(Item::result).collect();
    let mut detail_jobs = JoinSet::new();
    for (index, song) in songs.iter_mut().enumerate() {
        if let Some(video) = videos
            .iter()
            .find(|video| video.id == song.id)
            .and_then(Item::result)
        {
            song.artist = song.artist.take().or(video.artist);
            song.duration = song.duration.or(video.duration);
            song.thumbnail = song.thumbnail.take().or(video.thumbnail);
        }
        if (song.artist.is_none() || song.thumbnail.is_none()) && detail_jobs.len() < 3 {
            let executable = path.path.clone();
            let url = youtube_url(&song.id)?;
            detail_jobs.spawn(async move {
                let found = run_search(&executable, &url, false, "1").await.ok()?;
                let item = found.first()?.result()?;
                Some((index, item))
            });
        }
    }
    while let Some(job) = detail_jobs.join_next().await {
        if let Ok(Some((index, item))) = job {
            let song = &mut songs[index];
            if song.id == item.id {
                song.artist = item.artist.or_else(|| song.artist.take());
                song.duration = item.duration.or(song.duration);
                song.thumbnail = item.thumbnail.or_else(|| song.thumbnail.take());
            }
        }
    }
    let videos = videos
        .iter()
        .filter(|item| item.is_music_video())
        .filter_map(Item::result)
        .collect();
    Ok(rank(query, songs, videos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_search_preserves_the_query_and_only_accepts_real_video_links() {
        let url = music_url("Дарую & suanoo #songs").unwrap();
        assert_eq!(url.fragment(), Some("songs"));
        assert_eq!(url.query_pairs().next().unwrap().1, "Дарую & suanoo #songs");
        assert_eq!(
            direct_video("https://youtu.be/0PgTtkJc5ro?t=2").as_deref(),
            Some("0PgTtkJc5ro")
        );
        assert!(direct_video("https://youtube.com.invalid/watch?v=0PgTtkJc5ro").is_none());
    }

    #[test]
    fn promotes_matching_song_and_filters_unrelated_videos() {
        let item = |id: &str, title: &str, artist: &str| SearchResult {
            id: id.into(),
            title: title.into(),
            artist: Some(artist.into()),
            duration: None,
            thumbnail: None,
        };
        let results = rank(
            "Дарую suanoo",
            vec![
                item("related", "хэйкор", "Suanoo"),
                item("song", "дарую", "Suanoo"),
            ],
            vec![
                item("other", "Дарую", "Канал"),
                item("song", "дарую", "Suanoo"),
            ],
        );
        assert_eq!(
            results
                .iter()
                .map(|result| result.id.as_str())
                .collect::<Vec<_>>(),
            ["song", "related"]
        );
        assert!(!Item {
            title: Some("Реакция на official music video".into()),
            duration: Some(180.0),
            ..Item::default()
        }
        .is_music_video());
    }
}
