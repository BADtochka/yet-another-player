pub(crate) mod info;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod media;
mod preview;
pub mod source;
#[cfg(target_os = "windows")]
mod taskbar;
mod transitions;

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use rand::{seq::SliceRandom, Rng};
use rodio::{
    cpal::BufferSize, mixer::Mixer, ChannelCount, DeviceSinkBuilder, MixerDeviceSink, Player,
    Sample, SampleRate, Source,
};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tempfile::TempPath;
use tokio::sync::oneshot;

pub(crate) use preview::StartError as PreviewStartError;
use preview::{Preview, PreviewSnapshot};
pub(crate) use transitions::{analyze_pair, PairAnalysis, PairSettings, PlaylistMix};
use transitions::{
    Envelopes, FrameMix, Mode as TransitionMode, TransitionPreview, TransitionSettings, Transitions,
};

const TICK: Duration = Duration::from_millis(10);
const REPORT: Duration = Duration::from_millis(250);
const PREVIOUS_RESTART_POSITION: f64 = 5.0;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    path: String,
    replay_gain_db: Option<f32>,
}

impl Track {
    pub(crate) fn local(path: String, replay_gain_db: Option<f32>) -> Self {
        Self {
            path,
            replay_gain_db,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Normalization {
    Off,
    Quiet,
    Normal,
    Loud,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum RepeatMode {
    Off,
    All,
    One,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    Snapshot,
    InspectQueue {
        offset: usize,
    },
    Jump {
        index: usize,
        generation: u64,
    },
    Append {
        track: Track,
    },
    #[serde(skip_deserializing)]
    RemovePath {
        path: String,
    },
    ToggleShuffle,
    CycleRepeat,
    Configure {
        volume: f32,
        normalization: Normalization,
        #[serde(rename = "restartOnPrevious")]
        restart_on_previous: Option<bool>,
    },
    ConfigureTransitions {
        settings: TransitionSettings,
    },
    Volume {
        volume: f32,
    },
    #[serde(skip_deserializing)]
    AdjustVolume {
        delta: f32,
    },
    Queue {
        tracks: Vec<Track>,
        index: usize,
        position: f64,
        playing: bool,
        #[serde(skip)]
        restart_shuffle: bool,
        #[serde(skip)]
        playlist_mix: Option<PlaylistMix>,
    },
    Playing {
        playing: bool,
    },
    Seek {
        position: f64,
        path: String,
    },
    #[serde(skip_deserializing)]
    StartClip {
        id: u64,
        track: String,
        file: PathBuf,
        autoplay: bool,
    },
    StopClip {
        id: u64,
    },
    PreparePreview {
        title: String,
        artist: String,
    },
    #[serde(skip_deserializing)]
    StartTransitionPreview {
        id: u64,
        outgoing: PathBuf,
        incoming: PathBuf,
        settings: PairSettings,
    },
    ClosePreview {
        id: u64,
        #[serde(rename = "keepPaused", default)]
        keep_paused: bool,
    },
    PreviewPlaying {
        id: u64,
        playing: bool,
    },
    PreviewSeek {
        id: u64,
        position: f64,
    },
    Next,
    Previous,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    revision: u64,
    queue_generation: u64,
    queue_index: usize,
    queue_length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_paths: Option<Vec<String>>,
    next_path: Option<String>,
    clip_id: Option<u64>,
    timeline: u64,
    shuffle: bool,
    repeat: RepeatMode,
    path: Option<String>,
    playing: bool,
    position: f64,
    duration: Option<f64>,
    volume: f32,
    error: Option<String>,
    preview: Option<PreviewSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transition: Option<TransitionPreview>,
}

struct BufferedAudio {
    samples: Consumer<Sample>,
    ended: Arc<AtomicBool>,
    consumed: Arc<AtomicU64>,
    rate: SampleRate,
    channels: ChannelCount,
    channel: u16,
    silence: bool,
    envelopes: Arc<Envelopes>,
    mix: FrameMix,
    low: [f32; 32],
}

impl Iterator for BufferedAudio {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        // Consume a whole interleaved frame or emit a whole silent frame.
        if self.channel == 0 {
            let frame = self.consumed.load(Relaxed);
            self.mix = if let Some(envelope) = self.envelopes.outgoing.get() {
                if envelope.finished(frame) {
                    return None;
                }
                envelope.mix(frame)
            } else if let Some(envelope) = self
                .envelopes
                .incoming
                .get()
                .filter(|fade| !fade.finished(frame))
            {
                envelope.mix(frame)
            } else {
                FrameMix::default()
            };
            self.silence = self.samples.slots() < usize::from(self.channels.get());
            if self.silence && self.ended.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
        }

        let sample = if self.silence {
            0.0
        } else {
            let sample = self.samples.pop().ok()?;
            self.mix
                .sample(sample, &mut self.low[usize::from(self.channel)])
        };
        self.channel += 1;
        if self.channel == self.channels.get() {
            self.channel = 0;
            if !self.silence {
                self.consumed.fetch_add(1, Relaxed);
            }
        }
        Some(sample)
    }
}

impl Source for BufferedAudio {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct Playback {
    player: Player,
    decoder: Box<dyn Source + Send>,
    samples: Producer<Sample>,
    ended: Arc<AtomicBool>,
    consumed: Arc<AtomicU64>,
    envelopes: Arc<Envelopes>,
    rate: SampleRate,
    channels: ChannelCount,
    anchor: f64,
}

impl Playback {
    fn connect(mixer: &Mixer, ready: DecoderReady) -> Result<Self, String> {
        let DecoderReady {
            decoder, position, ..
        } = ready;
        let rate = decoder.sample_rate();
        let channels = decoder.channels();
        if rate.get() > 768_000 || channels.get() > 32 {
            return Err("Параметры аудио не поддерживаются".into());
        }
        let capacity = ((rate.get() as usize / 10).max(1)) * usize::from(channels.get());
        let (samples, consumer) = RingBuffer::new(capacity);
        let ended = Arc::new(AtomicBool::new(false));
        let consumed = Arc::new(AtomicU64::new(0));
        let envelopes = Arc::new(Envelopes::default());
        let (player, output) = Player::new();
        player.pause();

        let mut playback = Self {
            player,
            decoder,
            samples,
            ended: Arc::clone(&ended),
            consumed: Arc::clone(&consumed),
            envelopes: Arc::clone(&envelopes),
            anchor: position,
            rate,
            channels,
        };
        playback.fill();
        if playback.samples.slots() == capacity {
            return Err("Нет аудиоданных в выбранной позиции".into());
        }

        playback.player.append(BufferedAudio {
            samples: consumer,
            ended,
            consumed,
            rate,
            channels,
            channel: 0,
            silence: false,
            envelopes,
            mix: FrameMix::default(),
            low: [0.0; 32],
        });
        // The mixer reads the source format as soon as it connects.
        mixer.add(output);
        Ok(playback)
    }

    fn fill(&mut self) {
        if self.ended.load(Relaxed) {
            return;
        }
        let channels = usize::from(self.channels.get());
        let frames = self.samples.slots() / channels;
        for _ in 0..frames {
            for _ in 0..channels {
                let Some(sample) = self.decoder.next() else {
                    self.ended.store(true, std::sync::atomic::Ordering::Release);
                    return;
                };
                if self.samples.push(sample).is_err() {
                    return;
                }
            }
        }
    }

    fn position(&self) -> f64 {
        self.anchor + self.consumed.load(Relaxed) as f64 / f64::from(self.rate.get())
    }
}

struct DecoderReady {
    decoder: Box<dyn Source + Send>,
    position: f64,
    duration: Option<f64>,
}

fn open_decoder(path: &Path, position: f64) -> Result<DecoderReady, String> {
    let mut decoder = source::open(path).map_err(|error| {
        eprintln!("Audio decoder: {error}");
        "Формат трека не поддерживается или файл повреждён"
    })?;
    let duration = decoder.total_duration().map(|value| value.as_secs_f64());
    let position = if position.is_finite() {
        position.clamp(
            0.0,
            duration.map_or(f64::MAX, |value| (value - 0.01).max(0.0)),
        )
    } else {
        0.0
    };
    let target = Duration::try_from_secs_f64(position).map_err(|_| "Некорректная позиция")?;
    if !target.is_zero() {
        decoder
            .try_seek(target)
            .map_err(|_| "Не удалось перемотать трек")?;
    }
    Ok(DecoderReady {
        decoder,
        position,
        duration,
    })
}

struct OutgoingPlayback {
    playback: Playback,
    path: String,
    replay_gain_db: Option<f32>,
}

struct Engine {
    roots: Arc<Mutex<Vec<PathBuf>>>,
    queue: Vec<Track>,
    source_len: usize,
    order: Vec<usize>,
    next_shuffle: Option<Vec<usize>>,
    previous_shuffle: Option<Vec<usize>>,
    index: usize,
    normalization: Normalization,
    restart_on_previous: bool,
    output: Option<MixerDeviceSink>,
    output_failed: Arc<AtomicBool>,
    playback: Option<Playback>,
    outgoing: Option<OutgoingPlayback>,
    transitions: Transitions,
    clip_path: Option<PathBuf>,
    latest_clip_id: u64,
    completed: Option<String>,
    started: Option<String>,
    selection_started: bool,
    preview: Option<Preview>,
    preview_sequence: u64,
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    selection: u64,
    snapshot: Snapshot,
}

impl Engine {
    fn new(roots: Arc<Mutex<Vec<PathBuf>>>) -> Self {
        Self {
            roots,
            queue: Vec::new(),
            source_len: 0,
            order: Vec::new(),
            next_shuffle: None,
            previous_shuffle: None,
            index: 0,
            normalization: Normalization::Off,
            restart_on_previous: true,
            output: None,
            output_failed: Arc::new(AtomicBool::new(false)),
            playback: None,
            outgoing: None,
            transitions: Transitions::default(),
            clip_path: None,
            latest_clip_id: 0,
            completed: None,
            started: None,
            selection_started: false,
            preview: None,
            preview_sequence: 0,
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
            selection: 0,
            snapshot: Snapshot {
                revision: 0,
                queue_generation: 0,
                queue_index: 0,
                queue_length: 0,
                queue_paths: None,
                next_path: None,
                clip_id: None,
                timeline: 0,
                shuffle: false,
                repeat: RepeatMode::Off,
                path: None,
                playing: false,
                position: 0.0,
                duration: None,
                volume: 0.1,
                error: None,
                preview: None,
                transition: None,
            },
        }
    }

    fn current_track(&self) -> Option<&Track> {
        self.order
            .get(self.index)
            .and_then(|index| self.queue.get(*index))
    }

    fn set_queue(&mut self, tracks: Vec<Track>, selected: usize, restart_shuffle: bool) {
        if !restart_shuffle && self.snapshot.shuffle && self.source_len == tracks.len() {
            let existing = &self.queue[..self.source_len];
            if existing
                .iter()
                .zip(&tracks)
                .all(|(old, new)| old.path == new.path)
            {
                self.select_shuffled_track(tracks, selected);
                return;
            }

            // Sorting the same source changes its list order, but not its playback order.
            let mut by_path = HashMap::<_, VecDeque<_>>::new();
            for (index, track) in existing.iter().enumerate() {
                by_path
                    .entry(track.path.as_str())
                    .or_default()
                    .push_back(index);
            }
            let reordered = tracks
                .iter()
                .map(|track| by_path.get_mut(track.path.as_str())?.pop_front())
                .collect::<Option<Vec<_>>>();
            if let Some(reordered) = reordered {
                let mut indices = vec![0; tracks.len()];
                for (new, old) in reordered.into_iter().enumerate() {
                    indices[old] = new;
                }
                for order in std::iter::once(&mut self.order)
                    .chain(self.next_shuffle.iter_mut())
                    .chain(self.previous_shuffle.iter_mut())
                {
                    for index in order {
                        if let Some(new) = indices.get(*index) {
                            *index = *new;
                        }
                    }
                }
                self.select_shuffled_track(tracks, selected);
                return;
            }
        }
        let pending = self
            .order
            .iter()
            .skip(self.index + 1)
            .take_while(|&&index| index >= self.source_len)
            .map(|&index| self.queue[index].clone())
            .collect::<Vec<_>>();
        self.source_len = tracks.len();
        self.queue = tracks;
        self.order.clear();
        self.reorder(selected);
        if !pending.is_empty() {
            self.queue.extend(pending);
            let next = self.index + 1;
            self.order
                .splice(next..next, self.source_len..self.queue.len());
        }
    }

    fn select_shuffled_track(&mut self, tracks: Vec<Track>, selected: usize) {
        for (old, new) in self.queue.iter_mut().zip(tracks) {
            *old = new;
        }
        if let Some(index) = self.order.iter().position(|entry| *entry == selected) {
            self.index = index;
        }
    }

    fn reorder(&mut self, current: usize) {
        let pending = self
            .order
            .iter()
            .skip(self.index + 1)
            .copied()
            .filter(|&index| index >= self.source_len && index < self.queue.len())
            .collect::<Vec<_>>();
        self.order = (0..self.queue.len()).collect();
        if self.snapshot.shuffle && !self.order.is_empty() {
            self.order.swap(0, current);
            self.order[1..].shuffle(&mut rand::rng());
            self.index = 0;
        } else {
            self.index = current;
        }
        if !pending.is_empty() {
            self.order.retain(|index| !pending.contains(index));
            if let Some(index) = self.order.iter().position(|&index| index == current) {
                self.index = index;
            }
            self.order.splice(self.index + 1..self.index + 1, pending);
        }
        self.queue_changed();
    }

    fn queue_changed(&mut self) {
        self.next_shuffle = None;
        self.previous_shuffle = None;
        self.snapshot.queue_generation += 1;
    }

    fn remove_path(&mut self, path: &str) {
        if !self.queue.iter().any(|track| track.path == path) {
            return;
        }
        let removing_current = self.current_track().is_some_and(|track| track.path == path);
        let mut retained = 0;
        let map: Vec<_> = self
            .queue
            .iter()
            .map(|track| {
                (track.path != path).then(|| {
                    let index = retained;
                    retained += 1;
                    index
                })
            })
            .collect();
        let before = self
            .order
            .iter()
            .take(self.index)
            .filter(|&&index| map[index].is_some())
            .count();
        self.source_len = map[..self.source_len].iter().flatten().count();
        self.queue.retain(|track| track.path != path);
        self.order = self.order.iter().filter_map(|&index| map[index]).collect();
        self.index = before.min(self.order.len().saturating_sub(1));
        self.queue_changed();
        if removing_current {
            let playing = self.snapshot.playing && before < self.order.len();
            self.close_preview(false);
            self.select(0.0, playing);
            if self.order.is_empty() {
                self.snapshot.error = None;
            }
        }
    }

    fn continues_shuffling(&self) -> bool {
        self.snapshot.shuffle && self.snapshot.repeat == RepeatMode::Off && self.order.len() > 1
    }

    fn prepare_next_shuffle(&mut self) {
        if self.next_shuffle.is_some()
            || !self.continues_shuffling()
            || self.index + 1 != self.order.len()
        {
            return;
        }
        let mut order = self.order.clone();
        order.shuffle(&mut rand::rng());
        let last = &self.queue[self.order[self.index]].path;
        if &self.queue[order[0]].path == last {
            let candidates = &order[1..];
            let start = rand::rng().random_range(0..candidates.len());
            if let Some(offset) = (0..candidates.len())
                .map(|offset| (start + offset) % candidates.len())
                .find(|&offset| &self.queue[candidates[offset]].path != last)
            {
                order.swap(0, offset + 1);
            }
        }
        self.next_shuffle = Some(order);
    }

    fn next_queue_track(&mut self) -> Option<usize> {
        self.prepare_next_shuffle();
        match self.snapshot.repeat {
            RepeatMode::One => self.order.get(self.index).copied(),
            RepeatMode::All if self.index + 1 == self.order.len() => self.order.first().copied(),
            RepeatMode::Off if self.continues_shuffling() && self.index + 1 == self.order.len() => {
                self.next_shuffle.as_ref()?.first().copied()
            }
            _ => self.order.get(self.index + 1).copied(),
        }
    }

    fn open(&mut self, position: f64, playing: bool) -> Result<(), String> {
        let track = self.current_track().ok_or("Очередь пуста")?;
        let path = match &self.clip_path {
            Some(path) => path.clone(),
            None => library_track_path(&track.path, &self.roots)?,
        };
        self.replace_playback(&path, position, playing)
    }

    fn replace_playback(
        &mut self,
        path: &Path,
        position: f64,
        playing: bool,
    ) -> Result<(), String> {
        let (playback, duration) = self.build_playback(path, position)?;
        self.outgoing.take();
        self.snapshot.position = playback.anchor;
        self.playback = Some(playback);
        self.snapshot.duration = duration;
        self.snapshot.playing = playing;
        self.snapshot.error = None;
        self.snapshot.timeline += 1;
        self.apply_volume();
        if let Some(playback) = &self.playback {
            if playing {
                playback.player.play();
            }
        }
        Ok(())
    }

    fn switch_source(&mut self, path: &Path, autoplay: bool) -> Result<(), String> {
        let playing = self.snapshot.playing;
        if let Some(playback) = &self.playback {
            playback.player.pause();
        }
        let position = self
            .playback
            .as_ref()
            .map_or(self.snapshot.position, Playback::position);
        let result = self.replace_playback(path, position, playing || autoplay);
        if result.is_err() && playing {
            if let Some(playback) = &self.playback {
                playback.player.play();
            }
        }
        result
    }

    fn build_playback(
        &mut self,
        path: &Path,
        position: f64,
    ) -> Result<(Playback, Option<f64>), String> {
        self.attach_decoder(open_decoder(path, position)?)
    }

    fn attach_decoder(&mut self, ready: DecoderReady) -> Result<(Playback, Option<f64>), String> {
        if self.output.is_none() {
            let failed = Arc::clone(&self.output_failed);
            self.output = Some(
                DeviceSinkBuilder::from_default_device()
                    .and_then(|builder| {
                        builder
                            .with_buffer_size(BufferSize::Default)
                            .with_error_callback(move |_| failed.store(true, Relaxed))
                            .open_stream()
                    })
                    .map_err(|error| {
                        eprintln!("Audio output: {error}");
                        "Устройство вывода звука недоступно"
                    })?,
            );
        }
        let output = self
            .output
            .as_ref()
            .ok_or("Устройство вывода звука недоступно")?;
        let duration = ready.duration;
        let playback = Playback::connect(output.mixer(), ready)?;
        Ok((playback, duration))
    }

    fn change_selection(&mut self) {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        {
            self.selection += 1;
        }
    }

    fn select(&mut self, position: f64, playing: bool) {
        self.change_selection();
        self.playback.take();
        self.outgoing.take();
        self.transitions.prepared = None;
        let path = self.current_track().map(|track| track.path.clone());
        if path != self.snapshot.path {
            self.clip_path = None;
            self.snapshot.clip_id = None;
        }
        self.snapshot.path = path;
        self.snapshot.position = position;
        self.snapshot.duration = None;
        self.snapshot.playing = false;
        self.selection_started = false;
        self.snapshot.error = self.open(position, playing).err();
    }

    fn enqueue_next(&mut self, track: Track) {
        let index = if self.order.is_empty() {
            0
        } else {
            let next = self.index + 1;
            next + self.order[next..]
                .iter()
                .take_while(|&&index| index >= self.source_len)
                .count()
        };
        self.order.insert(index, self.queue.len());
        self.queue.push(track);
        self.queue_changed();
    }

    fn apply_volume(&self) {
        self.apply_preview_volume();
        if let Some(playback) = &self.playback {
            let gain = self.current_track().and_then(|track| track.replay_gain_db);
            playback.player.set_volume(self.track_volume(gain));
        }
        if let Some(outgoing) = &self.outgoing {
            outgoing
                .playback
                .player
                .set_volume(self.track_volume(outgoing.replay_gain_db));
        }
    }

    fn track_volume(&self, gain: Option<f32>) -> f32 {
        let gain = gain.filter(|value| value.is_finite()).unwrap_or(0.0);
        let db = match self.normalization {
            Normalization::Off => -2.67,
            Normalization::Quiet => gain - 16.0,
            Normalization::Normal => gain - 10.67,
            Normalization::Loud => gain - 5.33,
        };
        (self.snapshot.volume * 10_f32.powf(db.clamp(-120.0, 120.0) / 20.0)).clamp(0.0, 1.0)
    }

    fn update_volume(&mut self, volume: f32) {
        if volume.is_finite() {
            self.snapshot.volume = volume.clamp(0.0, 1.0);
            self.apply_volume();
        }
    }

    fn move_track(&mut self, next: bool) {
        if self.advance_queue(next) {
            self.select(0.0, true);
        }
    }

    fn should_restart_on_previous(&self) -> bool {
        self.restart_on_previous
            && self.snapshot.path.is_some()
            && self
                .playback
                .as_ref()
                .map_or(self.snapshot.position, Playback::position)
                >= PREVIOUS_RESTART_POSITION
    }

    fn seek(&mut self, position: f64) {
        let position = self.snapshot.duration.map_or(position, |duration| {
            position.min((duration - 0.01).max(0.0))
        });
        self.close_preview(true);
        self.snapshot.error = self.open(position, self.snapshot.playing).err();
    }

    fn advance_queue(&mut self, next: bool) -> bool {
        let len = self.order.len();
        if len == 0 {
            return false;
        }
        if self.continues_shuffling() {
            if next && self.index + 1 == len {
                self.prepare_next_shuffle();
                if let Some(order) = self.next_shuffle.take() {
                    self.previous_shuffle = Some(std::mem::replace(&mut self.order, order));
                    self.snapshot.queue_generation += 1;
                }
            } else if !next && self.index == 0 {
                if let Some(order) = self.previous_shuffle.take() {
                    self.next_shuffle = Some(std::mem::replace(&mut self.order, order));
                    self.snapshot.queue_generation += 1;
                }
            }
        }
        self.index = if next {
            (self.index + 1) % len
        } else {
            (self.index + len - 1) % len
        };
        true
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Snapshot | Action::InspectQueue { .. } => {}
            Action::Jump { index, generation } => {
                if generation == self.snapshot.queue_generation && index < self.order.len() {
                    self.close_preview(false);
                    self.index = index;
                    self.select(0.0, true);
                }
            }
            Action::Append { track } => {
                if let Err(error) = library_track_path(&track.path, &self.roots) {
                    self.snapshot.error = Some(error.into());
                    return;
                }
                let empty = self.order.is_empty();
                self.enqueue_next(track);
                self.snapshot.error = None;
                if empty {
                    self.change_selection();
                    self.snapshot.path = self.current_track().map(|track| track.path.clone());
                    self.snapshot.position = 0.0;
                    self.snapshot.duration = None;
                    self.snapshot.playing = false;
                    self.selection_started = false;
                }
            }
            Action::RemovePath { path } => self.remove_path(&path),
            Action::ToggleShuffle => {
                let current = self.order.get(self.index).copied().unwrap_or(0);
                self.snapshot.shuffle = !self.snapshot.shuffle;
                self.reorder(current);
            }
            Action::CycleRepeat => {
                self.snapshot.repeat = match self.snapshot.repeat {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
            }
            Action::Configure {
                volume,
                normalization,
                restart_on_previous,
            } => {
                self.normalization = normalization;
                if let Some(enabled) = restart_on_previous {
                    self.restart_on_previous = enabled;
                }
                self.update_volume(volume);
            }
            Action::ConfigureTransitions { settings } => {
                self.transitions.settings = settings.validated();
                if self.transitions.mode() == TransitionMode::Off {
                    self.transitions.prepared = None;
                }
            }
            Action::Volume { volume } => self.update_volume(volume),
            Action::AdjustVolume { delta } => self.update_volume(self.snapshot.volume + delta),
            Action::Queue {
                tracks,
                index,
                position,
                playing,
                restart_shuffle,
                playlist_mix,
            } => {
                if index >= tracks.len() {
                    self.snapshot.error = Some("Трек отсутствует в очереди".into());
                    return;
                }
                self.close_preview(false);
                self.transitions.playlist = playlist_mix;
                self.set_queue(tracks, index, restart_shuffle);
                self.select(position, playing);
            }
            Action::Playing { playing } => {
                self.close_preview(false);
                self.set_local_playing(playing);
            }
            Action::Seek { position, path } => {
                if self.snapshot.path.as_deref() == Some(&path) && position.is_finite() {
                    self.seek(position);
                }
            }
            Action::StartClip {
                id,
                track,
                file,
                autoplay,
            } => {
                if id <= self.latest_clip_id || self.snapshot.path.as_deref() != Some(&track) {
                    return;
                }
                self.latest_clip_id = id;
                self.close_preview(true);
                match self.switch_source(&file, autoplay) {
                    Ok(()) => {
                        self.clip_path = Some(file);
                        self.snapshot.clip_id = Some(id);
                    }
                    Err(error) => self.snapshot.error = Some(error),
                }
            }
            Action::StopClip { id } => {
                self.latest_clip_id = self.latest_clip_id.max(id);
                if self.snapshot.clip_id != Some(id) {
                    return;
                }
                self.clip_path = None;
                self.snapshot.clip_id = None;
                let result = self
                    .snapshot
                    .path
                    .as_deref()
                    .ok_or("Очередь пуста")
                    .and_then(|path| library_track_path(path, &self.roots))
                    .map_err(str::to_owned)
                    .and_then(|path| self.switch_source(&path, false));
                if let Err(error) = result {
                    self.playback.take();
                    self.snapshot.playing = false;
                    self.snapshot.error = Some(error);
                }
            }
            Action::PreparePreview { title, artist } => self.prepare_preview(title, artist),
            Action::StartTransitionPreview {
                id,
                outgoing,
                incoming,
                settings,
            } => {
                if self.has_preview(id) && self.active_preview().is_none() {
                    self.snapshot.error = self
                        .preview_transition(&outgoing, &incoming, settings.into())
                        .err();
                }
            }
            Action::ClosePreview { id, keep_paused } if self.has_preview(id) => {
                self.close_preview(!keep_paused);
            }
            Action::PreviewPlaying { id, playing } if self.has_preview(id) => {
                self.set_preview_playing(playing)
            }
            Action::PreviewSeek { id, position } if self.has_preview(id) => {
                self.seek_preview(position)
            }
            Action::ClosePreview { .. }
            | Action::PreviewPlaying { .. }
            | Action::PreviewSeek { .. } => {}
            Action::Previous if self.should_restart_on_previous() => self.seek(0.0),
            Action::Next | Action::Previous => {
                self.close_preview(false);
                self.move_track(matches!(action, Action::Next));
            }
        }
    }

    fn set_local_playing(&mut self, playing: bool) {
        match &self.playback {
            Some(playback) => {
                if playing {
                    playback.player.play();
                } else {
                    playback.player.pause();
                }
                if let Some(outgoing) = &self.outgoing {
                    if playing {
                        outgoing.playback.player.play();
                    } else {
                        outgoing.playback.player.pause();
                    }
                }
                self.snapshot.playing = playing;
            }
            None if playing && !self.queue.is_empty() => {
                let position = self
                    .snapshot
                    .duration
                    .map_or(self.snapshot.position, |duration| {
                        if self.snapshot.position >= duration {
                            0.0
                        } else {
                            self.snapshot.position
                        }
                    });
                self.select(position, true);
            }
            _ => {}
        }
    }

    fn tick(&mut self) -> bool {
        if self.output_failed.swap(false, Relaxed) {
            self.close_preview(false);
            self.playback.take();
            self.outgoing.take();
            self.output.take();
            self.snapshot.playing = false;
            self.snapshot.error = Some("Устройство вывода отключилось. Нажмите «Воспроизвести» для повторного подключения.".into());
            return true;
        }
        if self.active_preview().is_some() {
            return self.tick_preview();
        }
        self.transitions.poll();
        if let Some(outgoing) = &mut self.outgoing {
            outgoing.playback.fill();
            if outgoing.playback.player.empty() {
                self.completed = self.outgoing.take().map(|outgoing| outgoing.path);
            }
        }
        if let Some(playback) = &mut self.playback {
            playback.fill();
            let position = playback.position();
            if self.snapshot.playing
                && !self.selection_started
                && playback.consumed.load(Relaxed) > 0
            {
                self.started = self.snapshot.path.clone();
                self.selection_started = true;
            }
            self.snapshot.position = self
                .snapshot
                .duration
                .map_or(position, |duration| position.min(duration));
            if self.snapshot.playing && playback.player.empty() {
                self.completed = self.snapshot.path.clone();
                self.outgoing.take();
                match self.snapshot.repeat {
                    RepeatMode::One => self.select(0.0, true),
                    RepeatMode::Off
                        if self.index + 1 == self.order.len() && !self.continues_shuffling() =>
                    {
                        self.playback.take();
                        self.snapshot.position =
                            self.snapshot.duration.unwrap_or(self.snapshot.position);
                        self.snapshot.playing = false;
                    }
                    _ => self.move_track(true),
                }
                return true;
            }
        }
        self.tick_transition()
    }

    fn publish(&mut self, app: &AppHandle) {
        self.snapshot.revision += 1;
        self.snapshot.queue_index = self.index;
        self.snapshot.queue_length = self.order.len();
        self.snapshot.next_path = self
            .next_queue_track()
            .and_then(|index| self.queue.get(index))
            .map(|track| track.path.clone());
        self.snapshot.transition = self.transition_preview();
        let _ = app.emit("audio:state", &self.snapshot);
    }
}

pub(crate) fn library_track_path(
    path: &str,
    roots: &Mutex<Vec<PathBuf>>,
) -> Result<PathBuf, &'static str> {
    let path = std::fs::canonicalize(path).map_err(|_| "Файл трека недоступен")?;
    if path.is_file()
        && roots
            .lock()
            .map_err(|_| "Медиатека недоступна")?
            .iter()
            .any(|root| path.starts_with(root))
    {
        Ok(path)
    } else {
        Err("Трек находится вне папок медиатеки")
    }
}

enum Request {
    Ui {
        action: Action,
        reply: tauri::async_runtime::Sender<Snapshot>,
        queue_request: Option<u64>,
    },
    #[cfg(target_os = "windows")]
    Taskbar(taskbar::Command),
    ClaimPreview {
        id: u64,
        reply: oneshot::Sender<Option<oneshot::Receiver<()>>>,
    },
    StartPreview {
        id: u64,
        file: TempPath,
        reply: oneshot::Sender<Result<(), preview::StartError>>,
    },
    LyricsChanged(String),
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    Media {
        event: playwire::Event,
        selection: u64,
    },
}

pub struct AudioEngine(Arc<mpsc::Sender<Request>>, Arc<AtomicU64>);

enum PlaybackEvent {
    Started(String),
    Completed(String),
}

impl AudioEngine {
    pub fn start(app: AppHandle, roots: Arc<Mutex<Vec<PathBuf>>>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Request>();
        let tx = Arc::new(tx);
        let (history_tx, history_rx) = mpsc::channel::<PlaybackEvent>();
        let catalog_app = app.clone();
        thread::Builder::new()
            .name("yap-playback-history".into())
            .spawn(move || {
                let library = catalog_app.state::<crate::library::Library>();
                for event in history_rx {
                    let result = match event {
                        PlaybackEvent::Started(path) => {
                            library.record_playback_start(&path).map(|recorded| {
                                if recorded {
                                    let _ = catalog_app.emit("library:playback-history", ());
                                }
                            })
                        }
                        PlaybackEvent::Completed(path) => library.record_play(&path).map(|path| {
                            if let Some(path) = path {
                                let _ = catalog_app.emit("library:play-count", path);
                            }
                        }),
                    };
                    if let Err(error) = result {
                        eprintln!("Could not record playback: {error}");
                    }
                }
            })?;
        let queue_sequence = Arc::new(AtomicU64::new(0));
        let latest_queue = Arc::clone(&queue_sequence);
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let mut media = media::start(app.clone(), Arc::clone(&roots), Arc::downgrade(&tx));
        #[cfg(target_os = "windows")]
        taskbar::install(&app, Arc::downgrade(&tx));
        let database = app.state::<crate::library::Library>().database_path().to_owned();
        thread::Builder::new()
            .name("yap-audio".into())
            .spawn(move || {
                let mut engine = Engine::new(roots);
                engine.transitions = Transitions::with_catalog(database);
                let mut last_report = Instant::now();
                #[cfg(target_os = "windows")]
                let mut taskbar_state = taskbar::State::default();
                loop {
                    let request = match rx.recv_timeout(TICK) {
                        Ok(request) => Some(request),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let handled = request.is_some();
                    let reply = match request {
                        Some(Request::Ui {
                            action,
                            reply,
                            queue_request,
                        }) => {
                            let inspect = match action {
                                Action::InspectQueue { offset } => Some(offset),
                                _ => None,
                            };
                            if queue_request
                                .is_none_or(|request| request == latest_queue.load(Relaxed))
                            {
                                engine.apply(action);
                            }
                            Some((reply, inspect))
                        }
                        #[cfg(target_os = "windows")]
                        Some(Request::Taskbar(command)) => {
                            engine.apply_taskbar(command);
                            None
                        }
                        Some(Request::ClaimPreview { id, reply }) => {
                            let _ = reply.send(engine.claim_preview(id));
                            continue;
                        }
                        Some(Request::StartPreview { id, file, reply }) => {
                            let _ = reply.send(engine.start_preview(id, file));
                            None
                        }
                        Some(Request::LyricsChanged(path)) => {
                            engine.transitions.invalidate_lyrics(&path);
                            None
                        }
                        #[cfg(any(
                            target_os = "linux",
                            target_os = "macos",
                            target_os = "windows"
                        ))]
                        Some(Request::Media { event, selection }) => {
                            engine.apply_media(&app, event, selection);
                            None
                        }
                        None => None,
                    };
                    let changed = engine.tick();
                    if let Some(path) = engine.started.take() {
                        let _ = history_tx.send(PlaybackEvent::Started(path));
                    }
                    if let Some(path) = engine.completed.take() {
                        let _ = history_tx.send(PlaybackEvent::Completed(path));
                    }
                    if changed
                        || handled
                        || (engine.is_playing() && last_report.elapsed() >= REPORT)
                    {
                        engine.publish(&app);
                        #[cfg(any(
                            target_os = "linux",
                            target_os = "macos",
                            target_os = "windows"
                        ))]
                        if let Some(media) = &mut media {
                            media.publish(&engine);
                        }
                        #[cfg(target_os = "windows")]
                        {
                            let state = taskbar::State::from_engine(&engine);
                            if state != taskbar_state {
                                taskbar_state = state;
                                taskbar::publish(&app, state);
                            }
                        }
                        last_report = Instant::now();
                    }
                    if let Some((reply, inspect)) = reply {
                        let mut snapshot = engine.snapshot.clone();
                        if let Some(offset) = inspect {
                            snapshot.queue_paths = Some(
                                engine
                                    .order
                                    .iter()
                                    .skip(offset)
                                    .take(80)
                                    .map(|index| engine.queue[*index].path.clone())
                                    .collect(),
                            );
                        }
                        let _ = reply.blocking_send(snapshot);
                    }
                }
            })?;
        Ok(Self(tx, queue_sequence))
    }

    pub(crate) fn reserve_queue(&self) -> u64 {
        self.1.fetch_add(1, Relaxed) + 1
    }

    pub(crate) fn lyrics_changed(&self, path: &str) {
        let _ = self.0.send(Request::LyricsChanged(path.to_owned()));
    }

    pub(crate) async fn control(&self, action: Action) -> Result<Snapshot, String> {
        if matches!(
            action,
            Action::Queue { .. }
                | Action::Jump { .. }
                | Action::Next
                | Action::Previous
                | Action::Playing { .. }
                | Action::PreparePreview { .. }
        ) {
            self.reserve_queue();
        }
        self.send(action, None).await
    }

    pub(crate) async fn adjust_volume(&self, delta: f32) -> Result<f32, String> {
        self.send(Action::AdjustVolume { delta }, None)
            .await
            .map(|snapshot| snapshot.volume)
    }

    pub(crate) async fn control_reserved_queue(
        &self,
        request: u64,
        action: Action,
    ) -> Result<Snapshot, String> {
        self.send(action, Some(request)).await
    }

    async fn send(&self, action: Action, queue_request: Option<u64>) -> Result<Snapshot, String> {
        let (reply, mut response) = tauri::async_runtime::channel(1);
        self.0
            .send(Request::Ui {
                action,
                reply,
                queue_request,
            })
            .map_err(|_| "Плеер недоступен")?;
        response
            .recv()
            .await
            .ok_or_else(|| "Плеер недоступен".into())
    }
}

#[tauri::command]
pub async fn audio_control(
    state: State<'_, AudioEngine>,
    action: Action,
) -> Result<Snapshot, String> {
    state.control(action).await
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    fn tracks() -> Vec<Track> {
        ["a", "b", "c", "d"]
            .into_iter()
            .map(|name| Track::local(name.into(), None))
            .collect()
    }

    #[test]
    fn previous_restarts_the_current_track_at_five_seconds_by_default() {
        let mut engine = Engine::new(Arc::default());
        engine.set_queue(tracks(), 2, false);
        engine.snapshot.path = Some("c".into());
        engine.snapshot.position = 4.99;
        assert!(!engine.should_restart_on_previous());

        engine.snapshot.position = 5.0;
        assert!(engine.should_restart_on_previous());
        engine.apply(Action::Previous);
        assert_eq!(engine.index, 2);
        assert_eq!(engine.snapshot.path.as_deref(), Some("c"));
    }

    #[test]
    fn previous_changes_tracks_after_five_seconds_when_restart_is_disabled() {
        let mut engine = Engine::new(Arc::default());
        engine.set_queue(tracks(), 2, false);
        engine.snapshot.path = Some("c".into());
        engine.snapshot.position = 10.0;
        engine.apply(
            serde_json::from_str(
                r#"{"type":"configure","volume":0.1,"normalization":"off","restartOnPrevious":false}"#,
            )
            .unwrap(),
        );

        engine.apply(Action::Previous);
        assert_eq!(engine.snapshot.path.as_deref(), Some("b"));
    }

    #[test]
    fn enqueue_places_requests_after_the_current_track_in_request_order() {
        let mut engine = Engine::new(Arc::default());
        engine.set_queue(tracks(), 1, false);
        engine.enqueue_next(Track::local("first".into(), None));
        engine.enqueue_next(Track::local("second".into(), None));
        let paths = |engine: &Engine| {
            engine
                .order
                .iter()
                .map(|&id| engine.queue[id].path.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(&engine), ["a", "b", "first", "second", "c", "d"]);
        assert!(engine.advance_queue(true));
        engine.enqueue_next(Track::local("third".into(), None));
        engine.apply(Action::ToggleShuffle);
        let upcoming = paths(&engine);
        assert_eq!(
            upcoming[engine.index..engine.index + 3],
            ["first", "second", "third"]
        );
    }

    #[test]
    fn choosing_a_track_in_the_same_shuffled_source_keeps_order_and_appended_tracks() {
        let mut engine = Engine::new(Arc::default());
        engine.apply(Action::ToggleShuffle);
        engine.set_queue(tracks(), 1, false);
        engine.queue.push(Track::local("extra".into(), None));
        engine.order.push(4);
        engine.queue_changed();
        let order = engine.order.clone();
        let generation = engine.snapshot.queue_generation;

        engine.set_queue(tracks(), 2, false);

        assert_eq!(engine.order, order);
        assert_eq!(engine.snapshot.queue_generation, generation);
        assert_eq!(engine.current_track().unwrap().path, "c");

        let order_paths = |engine: &Engine| {
            engine
                .order
                .iter()
                .map(|&index| engine.queue[index].path.clone())
                .collect::<Vec<_>>()
        };
        let original = order_paths(&engine);
        let mut reordered = tracks();
        reordered.reverse();
        engine.set_queue(reordered, 0, false);
        assert_eq!(engine.current_track().unwrap().path, "d");
        assert_eq!(order_paths(&engine), original);
        assert_eq!(engine.snapshot.queue_generation, generation);
    }

    #[test]
    fn a_new_search_queue_starts_with_the_selected_track_even_when_shuffled() {
        let mut engine = Engine::new(Arc::default());
        engine.apply(Action::ToggleShuffle);
        engine.set_queue(tracks(), 0, false);
        engine.index = engine.order.len() - 1;
        let selected = engine.order[engine.index];
        let generation = engine.snapshot.queue_generation;

        engine.set_queue(tracks(), selected, true);

        assert_eq!(engine.index, 0);
        assert_eq!(engine.order[0], selected);
        assert_eq!(engine.order.len(), 4);
        assert!(engine.snapshot.queue_generation > generation);
    }

    #[test]
    fn shuffle_continues_with_a_new_cycle_and_an_accurate_next_track() {
        let mut engine = Engine::new(Arc::default());
        assert_eq!(engine.snapshot.repeat, RepeatMode::Off);
        engine.apply(Action::ToggleShuffle);
        engine.set_queue(tracks(), 0, false);
        engine.index = engine.order.len() - 1;
        let previous = engine.current_track().unwrap().path.clone();
        let next = engine.next_queue_track().unwrap();
        assert_ne!(engine.queue[next].path, previous);

        assert!(engine.advance_queue(true));
        assert_eq!(
            engine.current_track().unwrap().path,
            engine.queue[next].path
        );
        let mut order = engine.order.clone();
        order.sort_unstable();
        assert_eq!(order, vec![0, 1, 2, 3]);

        assert!(engine.advance_queue(false));
        assert_eq!(engine.current_track().unwrap().path, previous);
        assert_eq!(engine.next_queue_track(), Some(next));
    }

    #[test]
    fn a_single_track_only_repeats_automatically_when_repeat_is_selected() {
        let mut engine = Engine::new(Arc::default());
        engine.apply(Action::ToggleShuffle);
        engine.set_queue(vec![Track::local("a".into(), None)], 0, false);
        assert_eq!(engine.next_queue_track(), None);
        engine.apply(Action::CycleRepeat);
        engine.apply(Action::CycleRepeat);
        assert_eq!(engine.next_queue_track(), Some(0));
    }
}
