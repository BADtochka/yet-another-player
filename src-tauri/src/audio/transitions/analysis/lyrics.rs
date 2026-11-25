use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::{library::read_transition_lyrics, track_lyrics::read_sidecar};

use super::Edge;

const LAST_LINE_ALLOWANCE: f64 = 5.0;

pub(super) fn instrumental_span(
    track: &Path,
    roots: &Mutex<Vec<PathBuf>>,
    database: Option<&Path>,
    duration: f64,
    edge: Edge,
) -> Option<f64> {
    let sidecar = read_sidecar(track, roots);
    let tags = database
        .and_then(|database| read_transition_lyrics(database, track).ok())
        .unwrap_or_default();
    let embedded = (!tags.embedded.is_empty()).then(|| tags.embedded.join("\n"));
    [sidecar, embedded, tags.fetched]
        .into_iter()
        .flatten()
        .find_map(|text| from_lrc(&text, tags.offset_ms, duration, edge))
}

fn from_lrc(text: &str, track_offset_ms: i32, duration: f64, edge: Edge) -> Option<f64> {
    if text.len() > crate::track_lyrics::MAX_LYRICS_BYTES {
        return None;
    }
    let offset_ms = text
        .lines()
        .find_map(|line| {
            let (key, value) = line
                .trim()
                .strip_prefix('[')?
                .strip_suffix(']')?
                .split_once(':')?;
            key.eq_ignore_ascii_case("offset")
                .then(|| value.trim().parse::<i32>().ok())
                .flatten()
        })
        .unwrap_or_default();
    let offset = (f64::from(offset_ms) + f64::from(track_offset_ms)) / 1000.0;
    let mut times = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}');
        let start = times.len();
        while let Some((time, following)) =
            rest.strip_prefix('[').and_then(|line| line.split_once(']'))
        {
            let Some(time) = timestamp(time) else { break };
            times.push((time + offset).max(0.0));
            rest = following;
        }
        let vocal = rest
            .trim()
            .trim_matches(['[', ']', '(', ')'])
            .trim()
            .to_lowercase();
        if !vocal.chars().any(char::is_alphanumeric)
            || [
                "instrumental",
                "intro",
                "outro",
                "break",
                "проигрыш",
                "вступление",
            ]
            .contains(&vocal.as_str())
        {
            times.truncate(start);
        }
    }
    times.sort_unstable_by(f64::total_cmp);
    times.dedup();
    let (&first, &last) = times.first().zip(times.last())?;
    if times.len() < 3 || last > duration || last - first < duration * 0.22 {
        return None;
    }
    match edge {
        Edge::Head => Some(first),
        // A timestamp marks the start of the final line, and some sources only time an excerpt.
        Edge::Tail if last >= duration * 0.45 => {
            Some((duration - last - LAST_LINE_ALLOWANCE).max(0.0))
        }
        Edge::Tail => None,
    }
}

fn timestamp(value: &str) -> Option<f64> {
    let (minutes, seconds) = value.split_once(':')?;
    let minutes = minutes.parse::<u32>().ok()?;
    let (seconds, fraction) = seconds.split_once(['.', ':']).unwrap_or((seconds, ""));
    let seconds = seconds.parse::<u8>().ok()?;
    if seconds >= 60 || fraction.len() > 3 || !fraction.bytes().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let fraction = fraction.parse::<u32>().unwrap_or(0) as f64 / 10_f64.powi(fraction.len() as i32);
    Some(f64::from(minutes) * 60.0 + f64::from(seconds) + fraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_offsets_and_ignores_instrumental_markers() {
        let lrc = "[offset:500]\n[00:00]♪\n[00:01][Instrumental]\n[00:22.50]First\n[00:45][01:10]Second\n[01:20]Final\n[01:40]Outro";
        assert_eq!(from_lrc(lrc, -2000, 120.0, Edge::Head), Some(21.0));
        assert_eq!(from_lrc(lrc, -2000, 120.0, Edge::Tail), Some(36.5));
    }

    #[test]
    fn an_early_excerpt_does_not_allow_trimming_the_tail() {
        let lrc = "[00:04]First\n[00:37]Second\n[00:54]Third";
        assert_eq!(from_lrc(lrc, 0, 180.0, Edge::Tail), None);
    }
}
