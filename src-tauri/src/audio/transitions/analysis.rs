mod lyrics;

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use serde::Serialize;

const WINDOW: f64 = 20.0;
pub(super) const MAX_WINDOW: f64 = 48.0;
const BINS_PER_SECOND: usize = 100;
const BIN_SECONDS: f64 = 1.0 / BINS_PER_SECOND as f64;
const WAVEFORM_BINS: usize = 48;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Edge {
    Head,
    Tail,
}

#[derive(Clone, Copy)]
pub(super) struct Beat {
    pub(super) period: f64,
    pub(super) phase: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Features {
    #[serde(skip)]
    pub(super) rms: f32,
    pub(super) silence: f64,
    pub(super) fade_seconds: Option<f64>,
    pub(super) hook: Option<f64>,
    bpm: Option<u16>,
    pub(super) window_seconds: f64,
    pub(super) duration: Option<f64>,
    #[serde(skip)]
    pub(super) vocal_free: Option<f64>,
    #[serde(skip)]
    pub(super) steady: Option<f64>,
    #[serde(skip)]
    pub(super) beat: Option<Beat>,
    waveform: Vec<u8>,
}

pub(super) fn analyze(
    path: &Path,
    edge: Edge,
    roots: &Mutex<Vec<PathBuf>>,
    database: Option<&Path>,
) -> Option<Features> {
    let mut decoder = super::super::source::open(path).ok()?;
    let duration = decoder
        .total_duration()
        .map(|duration| duration.as_secs_f64());
    let vocal_free = duration
        .and_then(|duration| lyrics::instrumental_span(path, roots, database, duration, edge));
    let window = vocal_free.unwrap_or(0.0).ceil().clamp(WINDOW, MAX_WINDOW);
    if edge == Edge::Tail {
        let offset = (duration? - window).max(0.0);
        if offset > 0.0 {
            decoder
                .try_seek(Duration::try_from_secs_f64(offset).ok()?)
                .ok()?;
        }
    }
    let rate = decoder.sample_rate().get();
    let channels = usize::from(decoder.channels().get());
    if rate > 192_000 || channels > 8 {
        return None;
    }
    let mut position = 0;
    let bin_count = window as usize * BINS_PER_SECOND;
    let mut bins = Vec::with_capacity(bin_count);
    for bin in 1..=bin_count {
        let end = (bin * rate as usize / BINS_PER_SECOND) * channels;
        let count = end - position;
        if count == 0 {
            continue;
        }
        let mut energy = 0.0;
        let mut read = 0;
        for sample in decoder.by_ref().take(count) {
            if sample.is_finite() {
                energy += sample.clamp(-1.0, 1.0).powi(2);
            }
            read += 1;
        }
        if read > 0 {
            bins.push((energy / read as f32).sqrt());
        }
        if read < count {
            break;
        }
        position = end;
    }
    let mut features = summarize(&bins, edge)?;
    features.duration = duration;
    features.vocal_free = vocal_free;
    Some(features)
}

fn summarize(bins: &[f32], edge: Edge) -> Option<Features> {
    let maximum = bins.iter().copied().reduce(f32::max)?;
    let floor = (maximum * 0.08).clamp(0.003, 0.025);
    let audible = |window: &[f32]| window.iter().filter(|&&value| value > floor).count() >= 3;
    let silence_bins = match edge {
        Edge::Head => bins.windows(5).position(audible),
        Edge::Tail => bins
            .windows(5)
            .rposition(audible)
            .map(|index| bins.len() - index - 5),
    }?;
    let silence = (silence_bins.saturating_sub(4) as f64 * BIN_SECONDS).min(2.5);
    let (fade_seconds, hook) = match edge {
        Edge::Tail => (outro_fade(&bins[..bins.len() - silence_bins]), None),
        Edge::Head => (None, intro_hook(bins, silence_bins)),
    };
    let energy = match edge {
        Edge::Head => &bins[silence_bins..(silence_bins + 400).min(bins.len())],
        Edge::Tail => {
            let end = bins.len() - silence_bins;
            &bins[end.saturating_sub(400)..end]
        }
    };
    let rms = rms(energy);
    let beat = estimate_beat(bins).map(|mut beat| {
        if edge == Edge::Tail {
            beat.phase -= bins.len() as f64 * BIN_SECONDS;
        }
        beat
    });
    let steady = beat.filter(|_| fade_seconds.is_none()).and_then(|_| {
        let audible = match edge {
            Edge::Head => &bins[silence_bins..],
            Edge::Tail => &bins[..bins.len() - silence_bins],
        };
        steady_section(audible, edge)
    });
    let waveform = (0..WAVEFORM_BINS)
        .map(|index| {
            let start = index * bins.len() / WAVEFORM_BINS;
            let end = ((index + 1) * bins.len() / WAVEFORM_BINS).max(start + 1);
            let peak = bins[start..end].iter().copied().fold(0.0, f32::max);
            (peak.sqrt() * 100.0).round() as u8
        })
        .collect();
    Some(Features {
        rms,
        silence,
        fade_seconds,
        hook,
        bpm: beat.map(|beat| (60.0 / beat.period).round() as u16),
        window_seconds: bins.len() as f64 * BIN_SECONDS,
        duration: None,
        vocal_free: None,
        steady,
        beat,
        waveform,
    })
}

fn steady_section(bins: &[f32], edge: Edge) -> Option<f64> {
    let mut levels: Vec<_> = bins.chunks_exact(BINS_PER_SECOND).map(rms).collect();
    if edge == Edge::Tail {
        levels.reverse();
    }
    let reference = rms(levels.get(..3)?);
    if reference < 0.025 {
        return None;
    }
    let similar = |level| (reference * 0.72..=reference * 1.38).contains(&level);
    if !levels[..3].iter().copied().all(similar) {
        return None;
    }
    // Ignore isolated level changes when measuring a regular phrase.
    let end = levels
        .windows(2)
        .position(|pair| !similar(pair[0]) && !similar(pair[1]))
        .unwrap_or(levels.len());
    (end >= 6).then_some(end as f64)
}

fn rms(values: &[f32]) -> f32 {
    (values.iter().map(|value| value * value).sum::<f32>() / values.len() as f32).sqrt()
}

fn upper_quartile(mut levels: Vec<f32>) -> Option<f32> {
    let index = levels.len().checked_sub(1)? * 3 / 4;
    Some(*levels.select_nth_unstable_by(index, f32::total_cmp).1)
}

fn outro_fade(bins: &[f32]) -> Option<f64> {
    const STEP: usize = BINS_PER_SECOND / 4;
    let blocks: Vec<_> = bins.rchunks_exact(STEP).map(rms).collect();
    if blocks.len() < 12 {
        return None;
    }
    let levels: Vec<_> = blocks.windows(4).map(rms).collect();
    let reference = upper_quartile(levels[4..].to_vec())?;
    if reference < 0.025 || levels[0] > reference * 0.40 {
        return None;
    }
    let rise = levels.iter().position(|&level| level >= reference * 0.75)?;
    let increasing = levels[..=rise]
        .windows(2)
        .filter(|pair| pair[1] + reference * 0.025 >= pair[0])
        .count();
    if rise < 5 || increasing * 4 < rise * 3 {
        return None;
    }
    Some((rise as f64 * 0.25 + 0.5).min(12.0))
}

fn intro_hook(bins: &[f32], first: usize) -> Option<f64> {
    const STEP: usize = BINS_PER_SECOND / 10;
    let end = (first + BINS_PER_SECOND * 8).min(bins.len());
    let levels: Vec<_> = bins[first..end].chunks_exact(STEP).map(rms).collect();
    let reference = upper_quartile(levels.clone())?;
    if reference < 0.025 {
        return None;
    }
    levels
        .windows(3)
        .take(35)
        .enumerate()
        .find(|(index, attack)| {
            let level = rms(attack);
            if attack[0] < (reference * 0.75).max(0.045) {
                return false;
            }
            if *index < 3 {
                return level >= (reference * 1.2).max(0.065);
            }
            let before = rms(&levels[index.saturating_sub(4)..*index]);
            level >= (reference * 0.85).max(0.055) && level >= before * 1.8
        })
        .map(|(index, _)| (first + index * STEP) as f64 * BIN_SECONDS)
}

fn estimate_beat(bins: &[f32]) -> Option<Beat> {
    if bins.len() < BINS_PER_SECOND * 4 {
        return None;
    }
    let mut onsets = vec![0.0; bins.len()];
    for index in 3..bins.len() {
        onsets[index] = (bins[index] - bins[index - 3]).max(0.0);
    }
    let mean = onsets.iter().sum::<f32>() / onsets.len() as f32;
    for onset in &mut onsets {
        *onset = (*onset - mean).max(0.0);
    }
    let energy = onsets.iter().map(|value| value * value).sum::<f32>();
    if energy < 0.005 {
        return None;
    }
    let correlation = |lag: usize| {
        onsets[lag..]
            .iter()
            .zip(&onsets)
            .map(|(right, left)| right * left)
            .sum::<f32>()
            / energy
    };
    let (lag, score) = (30..=100)
        .map(|lag| (lag, correlation(lag) - 0.001 * lag as f32))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    if score < 0.30 {
        return None;
    }
    let left = correlation(lag - 1);
    let center = correlation(lag);
    let right = correlation(lag + 1);
    let curvature = left + right - 2.0 * center;
    let correction = if curvature < -0.0001 {
        (0.5 * (left - right) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let period = lag as f64 + f64::from(correction);
    let (sin, cos) = onsets
        .into_iter()
        .enumerate()
        .filter(|(_, onset)| *onset > 0.0)
        .fold((0.0, 0.0), |(sin, cos), (index, weight)| {
            let (s, c) = (std::f64::consts::TAU * index as f64 / period).sin_cos();
            (sin + s * f64::from(weight), cos + c * f64::from(weight))
        });
    let phase = sin.atan2(cos).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * period;
    Some(Beat {
        period: period * BIN_SECONDS,
        phase: phase * BIN_SECONDS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_attacks_yield_a_tempo_and_boundary_silence() {
        let mut bins = vec![0.0; 1200];
        for start in (100..1100).step_by(50) {
            bins[start..start + 12].fill(0.35);
        }
        let head = summarize(&bins, Edge::Head).unwrap();
        let tail = summarize(&bins, Edge::Tail).unwrap();
        assert_eq!(head.bpm, Some(120));
        assert!(head.silence > 0.9 && head.silence < 1.0);
        assert!(tail.silence > 1.25 && tail.silence < 1.4);
    }

    #[test]
    fn sustained_audio_does_not_produce_a_beat_grid() {
        let result = summarize(&[0.15; 1200], Edge::Head).unwrap();
        assert!(result.beat.is_none());
    }

    #[test]
    fn finds_sustained_outro_fade_without_treating_a_clean_end_as_a_fade() {
        let mut bins = vec![0.27; 1200];
        for (index, level) in bins[700..].iter_mut().enumerate() {
            *level *= 1.0 - index as f32 / 500.0;
        }
        let fade = summarize(&bins, Edge::Tail).unwrap().fade_seconds.unwrap();
        assert!((2.5..5.5).contains(&fade));
        assert!(summarize(&[0.27; 1200], Edge::Tail)
            .unwrap()
            .fade_seconds
            .is_none());
    }

    #[test]
    fn detects_an_early_hook_after_a_quiet_intro() {
        let mut bins = vec![0.09; 1200];
        bins[..40].fill(0.0);
        bins[40..130].fill(0.025);
        bins[130..220].fill(0.24);
        let hook = summarize(&bins, Edge::Head).unwrap().hook.unwrap();
        assert!((1.1..1.5).contains(&hook));
    }

    #[test]
    fn audio_only_regular_sections_adjust_overlap_without_skipping_audio() {
        use super::super::{Preset, TransitionSettings};

        let mut bins = vec![0.10; 2_000];
        for start in (0..2_000).step_by(50) {
            bins[start..start + 12].fill(0.20);
        }
        for level in &mut bins[800..] {
            *level *= 1.7;
        }
        let head = summarize(&bins, Edge::Head).unwrap();
        assert_eq!(head.steady, Some(8.0));
        assert!(head.hook.is_none());
        let plan = TransitionSettings::playlist().plan(None, Some(&head));
        assert_eq!(
            (plan.preset, plan.seconds, plan.cue, plan.trim),
            (Preset::Blend, 8.0, 0.0, 0.0)
        );
    }

    #[test]
    fn chooses_a_musical_overlap_for_a_fade_followed_by_an_early_hook() {
        use super::super::{Preset, TransitionSettings};

        let mut tail = vec![0.29; 1200];
        for (index, level) in tail[700..].iter_mut().enumerate() {
            *level *= 1.0 - index as f32 / 500.0;
        }
        let mut head = vec![0.10; 1200];
        head[40..150].fill(0.36);
        let tail = summarize(&tail, Edge::Tail).unwrap();
        let head = summarize(&head, Edge::Head).unwrap();
        let fade_only = TransitionSettings::playlist().plan(Some(&tail), None);
        let with_hook = TransitionSettings::playlist().plan(Some(&tail), Some(&head));

        assert_eq!(fade_only.preset, Preset::Outro);
        assert_eq!(with_hook.preset, Preset::Hook);
        assert!(with_hook.seconds >= fade_only.seconds);
        assert!(with_hook.cue <= head.hook.unwrap());
    }
}
