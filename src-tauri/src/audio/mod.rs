#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod media;
mod preview;
pub mod source;

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use rand::seq::SliceRandom;
use rodio::{
    cpal::BufferSize, ChannelCount, DeviceSinkBuilder, MixerDeviceSink, Player, Sample, SampleRate,
    Source,
};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tempfile::TempPath;
use tokio::sync::oneshot;

pub(crate) use preview::StartError as PreviewStartError;
use preview::{Preview, PreviewSnapshot};

const TICK: Duration = Duration::from_millis(10);
const REPORT: Duration = Duration::from_millis(250);

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

#[derive(Clone, Copy, Serialize)]
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
    ToggleShuffle,
    CycleRepeat,
    Configure {
        volume: f32,
        normalization: Normalization,
    },
    Volume {
        volume: f32,
    },
    Queue {
        tracks: Vec<Track>,
        index: usize,
        position: f64,
        playing: bool,
    },
    Playing {
        playing: bool,
    },
    Seek {
        position: f64,
        path: String,
    },
    PreparePreview {
        title: String,
        artist: String,
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
    shuffle: bool,
    repeat: RepeatMode,
    path: Option<String>,
    playing: bool,
    position: f64,
    duration: Option<f64>,
    volume: f32,
    error: Option<String>,
    preview: Option<PreviewSnapshot>,
}

struct BufferedAudio {
    samples: Consumer<Sample>,
    ended: Arc<AtomicBool>,
    consumed: Arc<AtomicU64>,
    rate: SampleRate,
    channels: ChannelCount,
    channel: u16,
    silence: bool,
}

impl Iterator for BufferedAudio {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        // Consume a whole interleaved frame or emit a whole silent frame.
        if self.channel == 0 {
            self.silence = self.samples.slots() < usize::from(self.channels.get());
            if self.silence && self.ended.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
        }

        let sample = if self.silence {
            0.0
        } else {
            self.samples.pop().ok()?
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
    rate: SampleRate,
    channels: ChannelCount,
    anchor: f64,
}

impl Playback {
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

struct Engine {
    roots: Arc<Mutex<Vec<PathBuf>>>,
    queue: Vec<Track>,
    order: Vec<usize>,
    index: usize,
    normalization: Normalization,
    output: Option<MixerDeviceSink>,
    output_failed: Arc<AtomicBool>,
    playback: Option<Playback>,
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
            order: Vec::new(),
            index: 0,
            normalization: Normalization::Off,
            output: None,
            output_failed: Arc::new(AtomicBool::new(false)),
            playback: None,
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
                shuffle: false,
                repeat: RepeatMode::All,
                path: None,
                playing: false,
                position: 0.0,
                duration: None,
                volume: 0.1,
                error: None,
                preview: None,
            },
        }
    }

    fn current_track(&self) -> Option<&Track> {
        self.order
            .get(self.index)
            .and_then(|index| self.queue.get(*index))
    }

    fn reorder(&mut self, current: usize) {
        self.order = (0..self.queue.len()).collect();
        if self.snapshot.shuffle && !self.order.is_empty() {
            self.order.swap(0, current);
            self.order[1..].shuffle(&mut rand::rng());
            self.index = 0;
        } else {
            self.index = current;
        }
        self.snapshot.queue_generation += 1;
    }

    fn open(&mut self, position: f64, playing: bool) -> Result<(), String> {
        let track = self.current_track().ok_or("Очередь пуста")?;
        let path = library_track_path(&track.path, &self.roots)?;
        let (playback, duration) = self.build_playback(&path, position)?;
        self.snapshot.position = playback.anchor;
        self.playback = Some(playback);
        self.snapshot.duration = duration;
        self.snapshot.playing = playing;
        self.snapshot.error = None;
        self.apply_volume();
        if let Some(playback) = &self.playback {
            if playing {
                playback.player.play();
            }
        }
        Ok(())
    }

    fn build_playback(
        &mut self,
        path: &Path,
        position: f64,
    ) -> Result<(Playback, Option<f64>), String> {
        let mut decoder = source::open(path).map_err(|error| {
            eprintln!("Audio decoder: {error}");
            "Формат трека не поддерживается или файл повреждён"
        })?;
        let duration = decoder.total_duration().map(|value| value.as_secs_f64());
        let position = if position.is_finite() {
            position.clamp(0.0, duration.unwrap_or(f64::MAX))
        } else {
            0.0
        };
        let target = Duration::try_from_secs_f64(position).map_err(|_| "Некорректная позиция")?;
        if !target.is_zero() {
            decoder
                .try_seek(target)
                .map_err(|_| "Не удалось перемотать трек")?;
        }

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
        let rate = decoder.sample_rate();
        let channels = decoder.channels();
        if rate.get() > 768_000 || channels.get() > 32 {
            return Err("Параметры аудио не поддерживаются".into());
        }
        let capacity = ((rate.get() as usize / 10).max(1)) * usize::from(channels.get());
        let (samples, consumer) = RingBuffer::new(capacity);
        let ended = Arc::new(AtomicBool::new(false));
        let consumed = Arc::new(AtomicU64::new(0));
        let player = Player::connect_new(output.mixer());
        player.pause();

        let mut playback = Playback {
            player,
            decoder,
            samples,
            ended: Arc::clone(&ended),
            consumed: Arc::clone(&consumed),
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
        });
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
        self.snapshot.path = self.current_track().map(|track| track.path.clone());
        self.snapshot.position = position;
        self.snapshot.duration = None;
        self.snapshot.playing = false;
        self.snapshot.error = self.open(position, playing).err();
    }

    fn apply_volume(&self) {
        self.apply_preview_volume();
        let Some(playback) = &self.playback else {
            return;
        };
        let gain = self
            .current_track()
            .and_then(|track| track.replay_gain_db)
            .filter(|value| value.is_finite())
            .unwrap_or(0.0);
        let db = match self.normalization {
            Normalization::Off => -2.67,
            Normalization::Quiet => gain - 16.0,
            Normalization::Normal => gain - 10.67,
            Normalization::Loud => gain - 5.33,
        };
        let volume =
            (self.snapshot.volume * 10_f32.powf(db.clamp(-120.0, 120.0) / 20.0)).clamp(0.0, 1.0);
        playback.player.set_volume(volume);
    }

    fn update_volume(&mut self, volume: f32) {
        if volume.is_finite() {
            self.snapshot.volume = volume.clamp(0.0, 1.0);
            self.apply_volume();
        }
    }

    fn move_track(&mut self, next: bool) {
        let len = self.queue.len();
        if len == 0 {
            return;
        }
        self.index = if next {
            (self.index + 1) % len
        } else {
            (self.index + len - 1) % len
        };
        self.select(0.0, true);
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
            } => {
                self.normalization = normalization;
                self.update_volume(volume);
            }
            Action::Volume { volume } => self.update_volume(volume),
            Action::Queue {
                tracks,
                index,
                position,
                playing,
            } => {
                if index >= tracks.len() {
                    self.snapshot.error = Some("Трек отсутствует в очереди".into());
                    return;
                }
                self.close_preview(false);
                self.queue = tracks;
                self.reorder(index);
                self.select(position, playing);
            }
            Action::Playing { playing } => {
                self.close_preview(false);
                self.set_local_playing(playing);
            }
            Action::Seek { position, path } => {
                if self.snapshot.path.as_deref() == Some(&path) && position.is_finite() {
                    let position = self.snapshot.duration.map_or(position, |duration| {
                        position.min((duration - 0.01).max(0.0))
                    });
                    self.close_preview(true);
                    self.snapshot.error = self.open(position, self.snapshot.playing).err();
                }
            }
            Action::PreparePreview { title, artist } => self.prepare_preview(title, artist),
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
            self.output.take();
            self.snapshot.playing = false;
            self.snapshot.error = Some("Устройство вывода отключилось. Нажмите «Воспроизвести» для повторного подключения.".into());
            return true;
        }
        if self.active_preview().is_some() {
            return self.tick_preview();
        }
        if let Some(playback) = &mut self.playback {
            playback.fill();
            let position = playback.position();
            self.snapshot.position = self
                .snapshot
                .duration
                .map_or(position, |duration| position.min(duration));
            if self.snapshot.playing && playback.player.empty() {
                match self.snapshot.repeat {
                    RepeatMode::One => self.select(0.0, true),
                    RepeatMode::Off if self.index + 1 == self.order.len() => {
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
        false
    }

    fn publish(&mut self, app: &AppHandle) {
        self.snapshot.revision += 1;
        self.snapshot.queue_index = self.index;
        self.snapshot.queue_length = self.order.len();
        let next_index = match self.snapshot.repeat {
            RepeatMode::One => self.index,
            RepeatMode::All if self.index + 1 == self.order.len() => 0,
            _ => self.index + 1,
        };
        self.snapshot.next_path = self
            .order
            .get(next_index)
            .and_then(|index| self.queue.get(*index))
            .map(|track| track.path.clone());
        let _ = app.emit("audio:state", &self.snapshot);
    }
}

fn library_track_path(path: &str, roots: &Mutex<Vec<PathBuf>>) -> Result<PathBuf, &'static str> {
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
    ClaimPreview {
        id: u64,
        reply: oneshot::Sender<Option<oneshot::Receiver<()>>>,
    },
    StartPreview {
        id: u64,
        file: TempPath,
        reply: oneshot::Sender<Result<(), preview::StartError>>,
    },
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    Media {
        event: playwire::Event,
        selection: u64,
    },
}

pub struct AudioEngine(Arc<mpsc::Sender<Request>>, Arc<AtomicU64>);

impl AudioEngine {
    pub fn start(app: AppHandle, roots: Arc<Mutex<Vec<PathBuf>>>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Request>();
        let tx = Arc::new(tx);
        let queue_sequence = Arc::new(AtomicU64::new(0));
        let latest_queue = Arc::clone(&queue_sequence);
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let mut media = media::start(app.clone(), Arc::clone(&roots), Arc::downgrade(&tx));
        thread::Builder::new()
            .name("yap-audio".into())
            .spawn(move || {
                let mut engine = Engine::new(roots);
                let mut last_report = Instant::now();
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
                        Some(Request::ClaimPreview { id, reply }) => {
                            let _ = reply.send(engine.claim_preview(id));
                            continue;
                        }
                        Some(Request::StartPreview { id, file, reply }) => {
                            let _ = reply.send(engine.start_preview(id, file));
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
                    if engine.tick()
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
