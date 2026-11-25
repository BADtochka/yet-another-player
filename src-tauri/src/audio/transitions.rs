mod analysis;

use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::atomic::Ordering::Relaxed,
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{library_track_path, DecoderReady, Engine, OutgoingPlayback, RepeatMode};
use analysis::{analyze, Edge, Features, MAX_WINDOW};

const LOOKAHEAD: usize = 3;
const CACHE_CAPACITY: usize = 32;
const LOOKAHEAD_INTERVAL: Duration = Duration::from_secs(1);
const ANALYSIS_LEAD: f64 = 75.0;
const PRELOAD_LEAD: f64 = 3.0;
const MIN_OVERLAP: f64 = 0.12;
const MIN_INSTRUMENTAL_SECTION: f64 = 6.0;
const VOCAL_LEAD_IN: f64 = 1.25;

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TransitionSettings {
    pub(super) mode: Mode,
    pub(super) seconds: f64,
    preset: Preset,
    max_seconds: f64,
    intensity: f32,
    trim_silence: bool,
    align_beats: bool,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) enum Mode {
    #[default]
    Off,
    Smooth,
    Advanced,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Preset {
    #[default]
    Auto,
    Blend,
    Linear,
    Dip,
    Rise,
    BassSwap,
    LowPass,
    HighPass,
    Cut,
    Outro,
    Hook,
}

#[derive(Clone, Copy)]
pub(super) struct Plan {
    preset: Preset,
    pub(super) seconds: f64,
    pub(super) trim: f64,
    pub(super) cue: f64,
    intensity: f32,
}

impl From<PairSettings> for Plan {
    fn from(settings: PairSettings) -> Self {
        let settings = settings.validated();
        Self {
            preset: if settings.preset == Preset::Auto {
                Preset::Blend
            } else {
                settings.preset
            },
            seconds: settings.seconds,
            trim: settings.trim,
            cue: settings.cue,
            intensity: settings.intensity,
        }
    }
}

pub(super) fn fade_decks(from: &super::Playback, to: &super::Playback, plan: Plan) {
    let _ = to
        .envelopes
        .incoming
        .set(Envelope::new(plan, to.rate, 0, true));
    let frame = from.consumed.load(Relaxed);
    let _ = from
        .envelopes
        .outgoing
        .set(Envelope::new(plan, from.rate, frame, false));
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairSettings {
    pub(super) preset: Preset,
    pub(super) seconds: f64,
    pub(crate) trim: f64,
    pub(crate) cue: f64,
    pub(crate) intensity: f32,
}

impl PairSettings {
    pub(crate) fn validated(mut self) -> Self {
        self.seconds = finite_or(self.seconds, 5.0).clamp(0.2, 12.0);
        self.trim = finite_or(self.trim, 0.0).clamp(0.0, MAX_WINDOW - 0.2);
        self.cue = finite_or(self.cue, 0.0).clamp(0.0, MAX_WINDOW - 0.2);
        self.intensity = if self.intensity.is_finite() {
            self.intensity.clamp(0.0, 1.0)
        } else {
            0.7
        };
        self
    }
}

pub type PlaylistMix = HashMap<String, (String, PairSettings)>;

#[derive(Serialize)]
pub struct PairAnalysis {
    suggested: PairSettings,
    tail: Option<Features>,
    head: Option<Features>,
}

pub(crate) fn analyze_pair(
    from: &Path,
    to: &Path,
    roots: &Mutex<Vec<PathBuf>>,
    database: &Path,
) -> PairAnalysis {
    let tail = read_features(from, Edge::Tail, roots, Some(database));
    let head = read_features(to, Edge::Head, roots, Some(database));
    let plan = TransitionSettings::playlist().plan(tail.as_ref(), head.as_ref());
    PairAnalysis {
        suggested: PairSettings {
            preset: plan.preset,
            seconds: plan.seconds,
            trim: plan.trim,
            cue: plan.cue,
            intensity: plan.intensity,
        },
        tail,
        head,
    }
}

fn read_features(
    path: &Path,
    edge: Edge,
    roots: &Mutex<Vec<PathBuf>>,
    database: Option<&Path>,
) -> Option<Features> {
    std::panic::catch_unwind(|| analyze(path, edge, roots, database))
        .ok()
        .flatten()
}

impl TransitionSettings {
    fn playlist() -> Self {
        Self {
            mode: Mode::Advanced,
            ..Self::default()
        }
    }

    pub(super) fn validated(mut self) -> Self {
        self.seconds = finite_or(self.seconds, 5.0).clamp(1.0, 12.0);
        self.max_seconds = finite_or(self.max_seconds, 8.0).clamp(2.0, 12.0);
        self.intensity = if self.intensity.is_finite() {
            self.intensity.clamp(0.0, 1.0)
        } else {
            0.7
        };
        self
    }

    fn plan(self, tail: Option<&Features>, head: Option<&Features>) -> Plan {
        if self.mode == Mode::Smooth {
            return Plan {
                preset: Preset::Blend,
                seconds: self.seconds,
                trim: 0.0,
                cue: 0.0,
                intensity: 0.0,
            };
        }
        let beat_pair = tail
            .and_then(|features| features.beat)
            .zip(head.and_then(|features| features.beat))
            .filter(|(tail, head)| (tail.period / head.period - 1.0).abs() <= 0.028);
        let automatic = self.preset == Preset::Auto;
        let instrumental = |features: &Features| {
            features
                .vocal_free
                .filter(|&seconds| automatic && seconds >= MIN_INSTRUMENTAL_SECTION)
        };
        let intro = head.and_then(instrumental);
        let outro = tail.and_then(instrumental);
        let steady = [tail, head]
            .into_iter()
            .flatten()
            .filter_map(|features| features.steady)
            .reduce(f64::min);
        let room_for_mix = intro.is_some() || outro.is_some() || steady.is_some();
        let fade = tail
            .and_then(|features| features.fade_seconds)
            .filter(|_| outro.is_none());
        let hook = head
            .and_then(|features| features.hook)
            .filter(|_| intro.is_none());
        let energy = tail.zip(head);
        let preset = match self.preset {
            Preset::Auto if hook.is_some() => Preset::Hook,
            Preset::Auto if beat_pair.is_some() && fade.is_none() => Preset::BassSwap,
            Preset::Auto if fade.is_some() => Preset::Outro,
            Preset::Auto if room_for_mix => Preset::Blend,
            Preset::Auto if energy.is_some_and(|(a, b)| a.rms > 0.19 && b.rms > 0.19) => {
                Preset::Dip
            }
            Preset::Auto if energy.is_some_and(|(a, b)| b.rms > 2.0 * a.rms) => Preset::Rise,
            Preset::Auto => Preset::Blend,
            preset => preset,
        };
        let mut seconds: f64 = match preset {
            Preset::Cut => 0.20,
            Preset::Dip => 3.5,
            Preset::Rise => 5.0,
            Preset::BassSwap => 8.0,
            Preset::Blend if automatic && room_for_mix => steady.unwrap_or(8.0).min(8.0),
            Preset::Outro => fade.unwrap_or(5.0),
            Preset::Hook => (hook.unwrap_or(0.0).clamp(0.0, 2.0) + 1.5).max(fade.unwrap_or(0.0)),
            _ => 6.0,
        };
        let mut trim = 0.0;
        let mut cue = if self.trim_silence {
            trim = tail.map_or(0.0, |tail| tail.silence);
            let silence = head.map_or(0.0, |head| head.silence);
            hook.map_or(silence, |hook| silence.min((hook - 0.08).max(0.0)))
        } else {
            0.0
        };
        seconds = seconds.min(self.max_seconds);
        let aligned =
            beat_pair.filter(|_| self.align_beats && !matches!(preset, Preset::Cut | Preset::Hook));
        if let Some((tail, _)) = aligned.filter(|_| preset == Preset::BassSwap) {
            // Reserve a fraction of a beat for phase alignment at the boundary.
            seconds = [16.0, 8.0, 4.0]
                .map(|beats| beats * tail.period)
                .into_iter()
                .find(|duration| *duration < self.max_seconds)
                .unwrap_or(seconds);
        }
        if let Some((features, span)) = tail.zip(outro) {
            // Start the overlap after the last vocal phrase and leave the rest of the outro out.
            trim = trim.max((span.min(features.window_seconds) - seconds - VOCAL_LEAD_IN).max(0.0));
        }
        if let Some((features, span)) = head.zip(intro) {
            // Bring the next track to full volume before its first line.
            let desired = span.min(features.window_seconds) - seconds - VOCAL_LEAD_IN;
            let limit = features
                .duration
                .map_or(desired, |duration| duration - seconds * 2.0);
            cue = cue.max(desired.min(limit).max(0.0));
        }
        if let Some((tail, head)) = aligned {
            let midpoint = seconds * 0.5;
            let tail_phase = (tail.phase + trim + seconds).rem_euclid(tail.period);
            let head_phase = (head.phase - cue - midpoint).rem_euclid(head.period)
                + midpoint.rem_euclid(tail.period);
            let displacement = (tail_phase - head_phase).rem_euclid(tail.period);
            let later = tail.period - displacement;
            // Keep the planned overlap inside the measured instrumental section.
            seconds += if displacement <= later
                || seconds + later > self.max_seconds
                || intro.is_some()
                || outro.is_some()
            {
                -displacement
            } else {
                later
            };
        }
        Plan {
            preset,
            seconds,
            trim,
            cue,
            intensity: self.intensity,
        }
    }
}

fn finite_or(value: f64, default: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        default
    }
}

impl Default for TransitionSettings {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            seconds: 5.0,
            preset: Preset::Auto,
            max_seconds: 8.0,
            intensity: 0.7,
            trim_silence: true,
            align_beats: true,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TransitionPreview {
    from_path: String,
    to_path: String,
    preset: Preset,
    seconds: f64,
    trim: f64,
    cue: f64,
    status: AnalysisStatus,
    tail: Option<Features>,
    head: Option<Features>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum AnalysisStatus {
    Waiting,
    Analyzing,
    Ready,
    Unavailable,
}

#[derive(Default)]
pub(super) struct Envelopes {
    pub(super) incoming: OnceLock<Envelope>,
    pub(super) outgoing: OnceLock<Envelope>,
}

pub(super) struct Envelope {
    start: u64,
    frames: u64,
    incoming: bool,
    preset: Preset,
    intensity: f32,
    filter: Filter,
    alpha: f32,
    hook_frames: u64,
}

#[derive(Clone, Copy, Default)]
pub(super) enum Filter {
    #[default]
    None,
    LowPass,
    HighPass,
}

#[derive(Clone, Copy)]
pub(super) struct FrameMix {
    pub(super) gain: f32,
    filter: Filter,
    wet: f32,
    alpha: f32,
}

impl Default for FrameMix {
    fn default() -> Self {
        Self {
            gain: 1.0,
            filter: Filter::None,
            wet: 0.0,
            alpha: 0.0,
        }
    }
}

impl FrameMix {
    pub(super) fn sample(self, sample: f32, low: &mut f32) -> f32 {
        let filtered = match self.filter {
            Filter::None => return sample * self.gain,
            Filter::LowPass => {
                *low += self.alpha * (sample - *low);
                *low
            }
            Filter::HighPass => {
                *low += self.alpha * (sample - *low);
                sample - *low
            }
        };
        (sample + self.wet * (filtered - sample)) * self.gain
    }
}

impl Envelope {
    pub(super) fn new(plan: Plan, rate: NonZeroU32, start: u64, incoming: bool) -> Self {
        let (filter, cutoff) = match plan.preset {
            Preset::BassSwap => (Filter::HighPass, 180.0),
            Preset::LowPass => (Filter::LowPass, 900.0),
            Preset::HighPass => (Filter::HighPass, 950.0),
            Preset::Rise if incoming => (Filter::HighPass, 650.0),
            _ => (Filter::None, 0.0),
        };
        let rate = f64::from(rate.get());
        Self {
            start,
            frames: ((plan.seconds * rate).round() as u64).max(1),
            incoming,
            preset: plan.preset,
            intensity: plan.intensity,
            filter,
            alpha: (1.0 - (-std::f64::consts::TAU * cutoff / rate).exp()) as f32,
            hook_frames: (rate * 0.16).round().max(1.0) as u64,
        }
    }

    pub(super) fn finished(&self, frame: u64) -> bool {
        frame >= self.start + self.frames
    }

    pub(super) fn mix(&self, frame: u64) -> FrameMix {
        let elapsed = frame.saturating_sub(self.start) as f32;
        let t = (elapsed / self.frames as f32).min(1.0);
        let progress = if self.incoming { t } else { 1.0 - t };
        let power = || (progress * std::f32::consts::FRAC_PI_2).sin();
        let gain = match self.preset {
            Preset::Linear => progress,
            Preset::Dip => power().powi(2),
            Preset::Rise if self.incoming => power().powi(3),
            Preset::Rise => power().sqrt(),
            Preset::Outro if !self.incoming => power().sqrt(),
            Preset::Hook => {
                // Reveal the attack immediately even when the preceding outro is long.
                let attack = (elapsed / self.hook_frames.min(self.frames) as f32).min(1.0);
                if self.incoming {
                    (attack * std::f32::consts::FRAC_PI_2).sin()
                } else {
                    (attack * std::f32::consts::FRAC_PI_2).cos() * (1.0 - t) * 0.8 + progress * 0.2
                }
            }
            Preset::Cut => ((progress - 0.35) / 0.30).clamp(0.0, 1.0),
            _ => power(),
        };
        let wet = match self.preset {
            Preset::BassSwap => ((1.0 - progress - 0.32) / 0.36).clamp(0.0, 1.0),
            _ => (1.0 - progress).powi(2),
        };
        FrameMix {
            gain,
            filter: self.filter,
            wet: wet * self.intensity,
            alpha: self.alpha,
        }
    }
}

#[derive(PartialEq)]
pub(super) struct Preparation {
    pub(super) path: String,
    pub(super) cue: f64,
}

enum Job {
    Analyze { path: String, edge: Edge },
    Prepare(Preparation),
}

enum JobResult {
    Analysis {
        path: String,
        edge: Edge,
        features: Option<Features>,
    },
    Prepared {
        request: Preparation,
        decoder: Option<DecoderReady>,
    },
}

struct Worker {
    sender: mpsc::Sender<Job>,
    receiver: mpsc::Receiver<JobResult>,
}

impl Worker {
    fn start(roots: Arc<Mutex<Vec<PathBuf>>>, database: Option<PathBuf>) -> Option<Self> {
        let (sender, work) = mpsc::channel();
        let (done, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("yap-transition-analysis".into())
            .spawn(move || {
                for job in work {
                    let result = match job {
                        Job::Analyze { path, edge } => {
                            let features =
                                library_track_path(&path, &roots).ok().and_then(|path| {
                                    read_features(&path, edge, &roots, database.as_deref())
                                });
                            JobResult::Analysis {
                                path,
                                edge,
                                features,
                            }
                        }
                        Job::Prepare(request) => {
                            let decoder =
                                library_track_path(&request.path, &roots)
                                    .ok()
                                    .and_then(|path| {
                                        super::open_decoder(&path, request.cue)
                                            .or_else(|error| {
                                                if request.cue > 0.0 {
                                                    super::open_decoder(&path, 0.0)
                                                } else {
                                                    Err(error)
                                                }
                                            })
                                            .ok()
                                    });
                            JobResult::Prepared { request, decoder }
                        }
                    };
                    if done.send(result).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { sender, receiver })
    }
}

#[derive(Default)]
pub(super) struct Transitions {
    pub(super) settings: TransitionSettings,
    worker: Option<Worker>,
    worker_failed: bool,
    busy: bool,
    analyzing: Option<String>,
    last_analysis: Option<Instant>,
    cache: VecDeque<(String, Edge, Option<Features>)>,
    database: Option<PathBuf>,
    pub(super) playlist: Option<PlaylistMix>,
    pub(super) prepared: Option<(Preparation, Option<DecoderReady>)>,
}

impl Transitions {
    pub(super) fn with_catalog(database: PathBuf) -> Self {
        Self {
            database: Some(database),
            ..Self::default()
        }
    }

    pub(super) fn invalidate_lyrics(&mut self, path: &str) {
        self.cache.retain(|(cached, _, _)| cached != path);
        if self.analyzing.as_deref() == Some(path) {
            self.analyzing = None;
        }
    }

    pub(super) fn poll(&mut self) {
        let Some(worker) = self.worker.as_ref().filter(|_| self.busy) else {
            return;
        };
        match worker.receiver.try_recv() {
            Ok(result) => {
                self.busy = false;
                match result {
                    JobResult::Analysis {
                        path,
                        edge,
                        features,
                    } => {
                        if self.analyzing.take().is_none() {
                            return;
                        }
                        if self.cache.len() == CACHE_CAPACITY {
                            self.cache.pop_front();
                        }
                        self.cache.push_back((path, edge, features));
                        self.last_analysis = Some(Instant::now());
                    }
                    JobResult::Prepared { request, decoder } => {
                        if self.mode() != Mode::Off {
                            self.prepared = Some((request, decoder));
                        }
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.worker = None;
                self.worker_failed = true;
                self.busy = false;
                self.analyzing = None;
            }
        }
    }

    fn schedule(&mut self, job: Job, roots: &Arc<Mutex<Vec<PathBuf>>>) {
        if self.busy || self.worker_failed {
            return;
        }
        if self.worker.is_none() {
            self.worker = Worker::start(Arc::clone(roots), self.database.clone());
        }
        self.analyzing = match &job {
            Job::Analyze { path, .. } => Some(path.clone()),
            Job::Prepare(_) => None,
        };
        self.busy = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.sender.send(job).is_ok());
        self.worker_failed = !self.busy;
    }

    pub(super) fn prepare(&mut self, request: Preparation, roots: &Arc<Mutex<Vec<PathBuf>>>) {
        self.schedule(Job::Prepare(request), roots);
    }

    fn entry(&self, path: &str, edge: Edge) -> Option<&Option<Features>> {
        self.cache
            .iter()
            .rev()
            .find(|(cached, end, _)| cached == path && *end == edge)
            .map(|(_, _, features)| features)
    }

    pub(super) fn analyze_queue<'a>(
        &mut self,
        paths: impl Iterator<Item = &'a str>,
        roots: &Arc<Mutex<Vec<PathBuf>>>,
    ) {
        if self.busy || self.worker_failed {
            return;
        }
        let mut paths = paths.take(LOOKAHEAD + 1);
        let Some(mut from) = paths.next() else { return };
        for (index, to) in paths.enumerate() {
            if index > 0
                && self
                    .last_analysis
                    .is_some_and(|at| at.elapsed() < LOOKAHEAD_INTERVAL)
            {
                return;
            }
            for (path, edge) in [(from, Edge::Tail), (to, Edge::Head)] {
                if from != to && self.entry(path, edge).is_none() {
                    self.schedule(
                        Job::Analyze {
                            path: path.to_owned(),
                            edge,
                        },
                        roots,
                    );
                    return;
                }
            }
            from = to;
        }
    }

    pub(super) fn plan(&self, from: &str, to: &str) -> Plan {
        let custom = self.pair_settings(from, to);
        let mut settings = if self.playlist.is_some() {
            TransitionSettings::playlist()
        } else {
            self.settings
        };
        if let Some(custom) = custom {
            settings.preset = custom.preset;
            // A manual duration owns the overlap length for this pair.
            settings.align_beats = false;
        }
        let mut plan = settings.plan(
            self.entry(from, Edge::Tail).and_then(Option::as_ref),
            self.entry(to, Edge::Head).and_then(Option::as_ref),
        );
        if let Some(custom) = custom {
            plan.seconds = custom.seconds;
            plan.trim = custom.trim;
            plan.cue = custom.cue;
            plan.intensity = custom.intensity;
        }
        plan
    }

    pub(super) fn mode(&self) -> Mode {
        if self.playlist.is_some() {
            Mode::Advanced
        } else {
            self.settings.mode
        }
    }

    fn pair_settings(&self, from: &str, to: &str) -> Option<PairSettings> {
        let (target, settings) = self.playlist.as_ref()?.get(from)?;
        (target == to).then_some(*settings)
    }

    pub(super) fn preview(&self, from: &str, to: &str, analyzing: bool) -> TransitionPreview {
        let plan = self.plan(from, to);
        let tail = self.entry(from, Edge::Tail);
        let head = self.entry(to, Edge::Head);
        let status = match (tail, head) {
            (Some(None), Some(None)) => AnalysisStatus::Unavailable,
            (Some(_), Some(_)) => AnalysisStatus::Ready,
            _ if self.worker_failed => AnalysisStatus::Unavailable,
            _ if analyzing => AnalysisStatus::Analyzing,
            _ => AnalysisStatus::Waiting,
        };
        TransitionPreview {
            from_path: from.to_owned(),
            to_path: to.to_owned(),
            preset: plan.preset,
            seconds: plan.seconds,
            trim: plan.trim,
            cue: plan.cue,
            status,
            tail: tail.cloned().flatten(),
            head: head.cloned().flatten(),
        }
    }
}

impl Engine {
    pub(super) fn tick_transition(&mut self) -> bool {
        let mode = self.transitions.mode();
        if mode == Mode::Off
            || !self.snapshot.playing
            || self.clip_path.is_some()
            || self.outgoing.is_some()
            || self.snapshot.repeat == RepeatMode::One
        {
            return false;
        }
        let Some(remaining) = self
            .snapshot
            .duration
            .map(|duration| duration - self.snapshot.position)
        else {
            return false;
        };
        if remaining < MIN_OVERLAP || remaining > ANALYSIS_LEAD {
            return false;
        }
        let Some(next) = self.next_queue_track() else {
            return false;
        };
        let Some(current) = self.current_track() else {
            return false;
        };
        let incoming = &self.queue[next];
        if current.path == incoming.path {
            return false;
        }
        let mut plan = self.transitions.plan(&current.path, &incoming.path);
        if remaining - plan.trim < MIN_OVERLAP
            && self
                .transitions
                .pair_settings(&current.path, &incoming.path)
                .is_none()
        {
            plan.trim = 0.0;
        }
        let until_end = remaining - plan.trim;
        if until_end <= plan.seconds + PRELOAD_LEAD {
            let request = Preparation {
                path: incoming.path.clone(),
                cue: plan.cue,
            };
            if let Some((_, ready)) = self
                .transitions
                .prepared
                .as_mut()
                .filter(|(cached, _)| *cached == request)
            {
                if let Some(ready) = ready {
                    if let Some(duration) = ready.duration {
                        plan.seconds = plan.seconds.min((duration - ready.position) * 0.5);
                    }
                }
                if until_end <= plan.seconds && until_end >= MIN_OVERLAP {
                    if let Some(ready) = ready.take() {
                        return self.start_transition(ready, plan, until_end);
                    }
                }
            } else {
                self.transitions.prepare(request, &self.roots);
                return false;
            }
        }
        if mode == Mode::Advanced {
            let after_queue = match self.snapshot.repeat {
                RepeatMode::All => &self.order[..=self.index],
                _ => self.next_shuffle.as_deref().unwrap_or_default(),
            };
            let paths = self.order[self.index..]
                .iter()
                .chain(after_queue)
                .map(|&index| self.queue[index].path.as_str());
            self.transitions.analyze_queue(paths, &self.roots);
        }
        false
    }

    fn start_transition(&mut self, ready: DecoderReady, mut plan: Plan, until_end: f64) -> bool {
        let Some(track) = self.current_track() else {
            return false;
        };
        let path = track.path.clone();
        let replay_gain_db = track.replay_gain_db;
        plan.seconds = plan.seconds.min(until_end);
        if plan.seconds < MIN_OVERLAP {
            return false;
        }
        let Ok((incoming, duration)) = self.attach_decoder(ready) else {
            return false;
        };
        let Some(outgoing) = self.playback.take() else {
            return false;
        };
        fade_decks(&outgoing, &incoming, plan);
        self.advance_queue(true);
        self.change_selection();
        self.snapshot.path = self.current_track().map(|track| track.path.clone());
        self.snapshot.position = incoming.anchor;
        self.snapshot.duration = duration;
        self.snapshot.timeline += 1;
        self.snapshot.error = None;
        self.selection_started = false;
        self.transitions.prepared = None;
        self.outgoing = Some(OutgoingPlayback {
            playback: outgoing,
            path,
            replay_gain_db,
        });
        self.playback = Some(incoming);
        self.apply_volume();
        if let Some(playback) = &self.playback {
            playback.player.play();
        }
        true
    }

    pub(super) fn transition_preview(&self) -> Option<TransitionPreview> {
        if self.transitions.mode() != Mode::Advanced || self.clip_path.is_some() {
            return None;
        }
        let from = self.snapshot.path.as_deref()?;
        let to = self.snapshot.next_path.as_deref()?;
        if from == to {
            return None;
        }
        let analyzing = self.snapshot.playing
            && self
                .snapshot
                .duration
                .is_some_and(|duration| duration - self.snapshot.position <= ANALYSIS_LEAD);
        Some(self.transitions.preview(from, to, analyzing))
    }
}

#[cfg(test)]
mod tests {
    use rodio::{buffer::SamplesBuffer, mixer::mixer, ChannelCount, SampleRate};

    use super::{super::Playback, *};

    #[test]
    fn mixed_playbacks_fade_between_two_stereo_sources() {
        let rate = SampleRate::new(8_000).unwrap();
        let channels = ChannelCount::new(2).unwrap();
        let (mix, mut output) = mixer(channels, rate);
        let connect = |frame: [f32; 2]| {
            let samples = frame.repeat(16_000);
            Playback::connect(
                &mix,
                DecoderReady {
                    decoder: Box::new(SamplesBuffer::new(channels, rate, samples)),
                    position: 0.0,
                    duration: Some(2.0),
                },
            )
            .unwrap()
        };
        let mut outgoing = connect([0.5, 0.0]);
        let mut incoming = connect([0.0, 0.5]);
        let plan = Plan {
            preset: Preset::Linear,
            seconds: 1.0,
            trim: 0.0,
            cue: 0.0,
            intensity: 0.0,
        };
        assert!(outgoing
            .envelopes
            .outgoing
            .set(Envelope::new(plan, rate, 0, false))
            .is_ok());
        assert!(incoming
            .envelopes
            .incoming
            .set(Envelope::new(plan, rate, 0, true))
            .is_ok());
        outgoing.player.play();
        incoming.player.play();
        for frame in 0..=8_100 {
            if frame % 80 == 0 {
                outgoing.fill();
                incoming.fill();
            }
            let left = output.next().unwrap();
            let right = output.next().unwrap();
            let phase = (frame as f32 / 8_000.0).min(1.0);
            assert!(
                (left - 0.5 * (1.0 - phase)).abs() < 0.004,
                "{frame}, left {left}"
            );
            assert!(
                (right - 0.5 * phase).abs() < 0.004,
                "{frame}, right {right}"
            );
        }
        assert!(outgoing.player.empty());
        assert!(!incoming.player.empty());
    }

    #[test]
    fn playlist_override_only_applies_when_its_directed_pair_is_adjacent() {
        let settings = PairSettings {
            preset: Preset::Cut,
            seconds: 0.3,
            trim: 2.0,
            cue: 1.2,
            intensity: 0.4,
        };
        let transitions = Transitions {
            playlist: Some(HashMap::from([("a".into(), ("b".into(), settings))])),
            ..Transitions::default()
        };
        let plan = transitions.plan("a", "b");
        assert_eq!(
            (plan.preset, plan.seconds, plan.trim, plan.cue),
            (Preset::Cut, 0.3, 2.0, 1.2)
        );
        assert_eq!(transitions.plan("b", "a").preset, Preset::Blend);
        assert_eq!(transitions.plan("a", "c").preset, Preset::Blend);
    }

    #[test]
    fn an_early_hook_is_audible_during_a_long_outro() {
        let plan = Plan {
            preset: Preset::Hook,
            seconds: 8.0,
            trim: 0.0,
            cue: 0.0,
            intensity: 0.7,
        };
        let rate = SampleRate::new(8_000).unwrap();
        let incoming = Envelope::new(plan, rate, 0, true);
        let outgoing = Envelope::new(plan, rate, 0, false);
        let frame = 1_280;
        assert!(incoming.mix(frame).gain > 0.95);
        assert!(outgoing.mix(frame).gain < 0.25);
    }

    #[test]
    fn automatic_mix_uses_synchronized_instrumental_edges_in_queue_and_editor() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().canonicalize().unwrap();
        let roots = Mutex::new(vec![directory.clone()]);
        let database = directory.join("catalog.sqlite3");
        let _library = crate::library::Library::new(database.clone()).unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        let path = directory.join("outgoing.wav");
        let next = directory.join("incoming.wav");
        let duration = 120_u32;
        let data_bytes = duration * 8_000 * 2;
        let mut audio = Vec::new();
        audio.extend_from_slice(b"RIFF");
        audio.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        audio.extend_from_slice(
            b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data",
        );
        audio.extend_from_slice(&data_bytes.to_le_bytes());
        audio.extend([0x00, 0x20].into_iter().cycle().take(data_bytes as usize));
        for track in [&path, &next] {
            std::fs::write(track, &audio).unwrap();
        }
        std::fs::write(
            next.with_extension("lrc"),
            "[00:31]First\n[00:47]Second\n[01:18]Last",
        )
        .unwrap();
        connection
            .execute(
                "INSERT INTO track(id, path, stamp, size, metadata) VALUES (1, ?1, '', 0, ?2)",
                rusqlite::params![path.to_str(), r#"{"lyrics":[],"lyricsOffsetMs":1000}"#],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO fetched_lyrics (track_id, text) VALUES (1, ?1)",
                ["[00:31]First\n[00:47]Second\n[01:18]Last"],
            )
            .unwrap();

        let editor = analyze_pair(&path, &next, &roots, &database);
        let mut transitions = Transitions::default();
        let path = path.to_str().unwrap();
        let next = next.to_str().unwrap();
        transitions.settings = TransitionSettings::playlist();
        transitions
            .cache
            .push_back((path.into(), Edge::Tail, editor.tail));
        transitions
            .cache
            .push_back((next.into(), Edge::Head, editor.head));
        let plan = transitions.plan(path, next);
        assert!(plan.trim > 20.0 && plan.cue > 20.0);
        assert!(f64::from(duration) - plan.trim - plan.seconds >= 84.0);
        assert!(plan.cue + plan.seconds < 31.0);
        assert_eq!(
            (plan.trim, plan.cue),
            (editor.suggested.trim, editor.suggested.cue)
        );
        assert_eq!(editor.suggested, editor.suggested.validated());
    }

    #[test]
    fn smooth_mode_keeps_the_configured_time_and_plays_from_the_start() {
        let raw = r#"{"type":"configureTransitions","settings":{"mode":"smooth","seconds":4.5}}"#;
        let super::super::Action::ConfigureTransitions { settings } =
            serde_json::from_str(raw).unwrap()
        else {
            panic!("Unexpected audio action")
        };
        let plan = settings.validated().plan(None, None);

        assert_eq!(
            (plan.preset, plan.seconds, plan.cue, plan.trim),
            (Preset::Blend, 4.5, 0.0, 0.0)
        );
    }
}
