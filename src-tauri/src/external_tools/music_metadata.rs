pub(crate) fn split_credits(value: &str) -> impl Iterator<Item = &str> {
    value
        .split([',', ';'])
        .flat_map(|part| part.split(" feat. "))
        .flat_map(|part| part.split(" ft. "))
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn is_video_label(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "official music video"
            | "official video"
            | "official audio"
            | "official lyric video"
            | "official visualizer"
            | "music video"
            | "lyric video"
            | "lyrics video"
            | "visualizer"
            | "официальный клип"
            | "официальное видео"
            | "официальное аудио"
    )
}

pub(crate) fn has_clip_label(title: &str) -> bool {
    let is_clip = |label: &str| {
        matches!(
            label.trim().to_lowercase().as_str(),
            "official music video"
                | "official video"
                | "music video"
                | "официальный клип"
                | "официальное видео"
                | "клип"
        )
    };
    title.match_indices(['(', '[']).any(|(index, bracket)| {
        let end = if bracket == "(" { ')' } else { ']' };
        title[index + 1..]
            .split_once(end)
            .is_some_and(|(label, _)| is_clip(label))
    }) || title
        .rsplit_once(" | ")
        .is_some_and(|(_, label)| is_clip(label))
}

pub(crate) fn normalize(title: &str, artists: &mut Vec<String>) -> String {
    let mut title = title.trim();
    let mut video_label = false;
    loop {
        let label = title
            .strip_suffix(')')
            .and_then(|text| text.rsplit_once('('))
            .or_else(|| {
                title
                    .strip_suffix(']')
                    .and_then(|text| text.rsplit_once('['))
            })
            .or_else(|| title.rsplit_once(" | "));
        let Some((name, _)) =
            label.filter(|(name, label)| !name.trim().is_empty() && is_video_label(label))
        else {
            break;
        };
        title = name.trim_end();
        video_label = true;
    }

    if let Some((prefix, name)) = [" - ", " — ", " – "]
        .into_iter()
        .filter_map(|sep| title.split_once(sep))
        .min_by_key(|(prefix, _)| prefix.len())
    {
        let prefix = prefix.trim();
        let name = name.trim();
        let credits = split_credits(prefix).collect::<Vec<_>>();
        let agrees = !artists.is_empty()
            && artists.iter().all(|artist| {
                split_credits(artist).all(|artist| {
                    credits
                        .iter()
                        .any(|credit| credit.to_lowercase() == artist.to_lowercase())
                })
            });
        if !credits.is_empty() && !name.is_empty() && (video_label || agrees) {
            if artists.is_empty()
                || (agrees && (artists.len() == 1 || credits.len() > artists.len()))
            {
                *artists = credits.into_iter().map(str::to_owned).collect();
            }
            title = name;
        }
    }
    title.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_video_label_and_keeps_multiple_artist_credits() {
        let mut artists = vec!["madk1d, VILLIAN".to_owned()];
        assert_eq!(
            normalize(
                "madk1d, VILLIAN - 8 миля (official music video)",
                &mut artists
            ),
            "8 миля"
        );
        assert_eq!(artists, ["madk1d", "VILLIAN"]);
        assert_eq!(normalize("8 миля (Live)", &mut artists), "8 миля (Live)");
    }
}
