use std::{path::Path, time::Duration};

use serde::Serialize;
use tempfile::TempPath;
use tokio::sync::oneshot;

use super::{
    open_decoder,
    transitions::{fade_decks, Plan},
    AudioEngine, Engine, Playback, Request,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PreviewSnapshot {
    pub(super) id: u64,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) loading: bool,
    pub(super) playing: bool,
    pub(super) position: f64,
    pub(super) duration: Option<f64>,
}

pub(super) struct Preview {
    // Dropping the session cancels its pending download, including on a track change.
    _cancel_on_drop: oneshot::Sender<()>,
    cancellation: Option<oneshot::Receiver<()>>,
    audio: Option<PreviewAudio>,
    resume_local: bool,
}

struct PreviewAudio {
    playback: Playback,
    file: Option<TempPath>,
    mix: Option<MixPreview>,
}

struct MixPreview {
    incoming: Playback,
    plan: Plan,
    delay: f64,
    started: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StartError {
    Cancelled,
    Failed,
}

impl Engine {
    pub(super) fn prepare_preview(&mut self, title: String, artist: String) {
        self.close_preview(true);
        self.preview_sequence += 1;
        let (cancel, cancellation) = oneshot::channel();
        self.preview = Some(Preview {
            _cancel_on_drop: cancel,
            cancellation: Some(cancellation),
            audio: None,
            resume_local: false,
        });
        self.snapshot.preview = Some(PreviewSnapshot {
            id: self.preview_sequence,
            title: display_text(&title),
            artist: display_text(&artist),
            loading: true,
            playing: false,
            position: 0.0,
            duration: None,
        });
    }

    pub(super) fn has_preview(&self, id: u64) -> bool {
        self.snapshot
            .preview
            .as_ref()
            .is_some_and(|preview| preview.id == id)
    }

    pub(super) fn active_preview(&self) -> Option<&PreviewSnapshot> {
        self.snapshot
            .preview
            .as_ref()
            .filter(|preview| !preview.loading)
    }

    pub(super) fn is_playing(&self) -> bool {
        self.snapshot.playing || self.active_preview().is_some_and(|preview| preview.playing)
    }

    pub(super) fn claim_preview(&mut self, id: u64) -> Option<oneshot::Receiver<()>> {
        if !self.has_preview(id) {
            return None;
        }
        self.preview.as_mut()?.cancellation.take()
    }

    pub(super) fn start_preview(&mut self, id: u64, file: TempPath) -> Result<(), StartError> {
        if !self.has_preview(id) || self.active_preview().is_some() {
            return Err(StartError::Cancelled);
        }
        let (playback, duration) = self.build_playback(&file, 0.0).map_err(|error| {
            eprintln!("Audio preview: {error}");
            StartError::Failed
        })?;
        self.activate_preview(
            PreviewAudio {
                playback,
                file: Some(file),
                mix: None,
            },
            duration,
        );
        Ok(())
    }

    pub(super) fn preview_transition(
        &mut self,
        outgoing: &Path,
        incoming: &Path,
        mut plan: Plan,
    ) -> Result<(), String> {
        let mut tail = open_decoder(outgoing, 0.0)?;
        let head = open_decoder(incoming, plan.cue).or_else(|_| open_decoder(incoming, 0.0))?;
        let end = (tail
            .duration
            .ok_or("Не удалось определить длительность трека")?
            - plan.trim)
            .max(0.0);
        let incoming_available = head
            .duration
            .map_or(plan.seconds + 2.0, |duration| duration - head.position);
        plan.seconds = plan.seconds.min(end).min(incoming_available * 0.5);
        if plan.seconds < 0.12 {
            return Err("Для этого перехода недостаточно аудио".into());
        }
        let start = (end - plan.seconds - 2.0).max(0.0);
        let delay = end - plan.seconds - start;
        if start > 0.0 {
            let position =
                Duration::try_from_secs_f64(start).map_err(|_| "Некорректная позиция")?;
            tail.decoder
                .try_seek(position)
                .map_err(|_| "Не удалось перемотать трек для прослушивания")?;
            tail.position = start;
        }
        let (playback, _) = self.attach_decoder(tail)?;
        let (incoming, _) = self.attach_decoder(head)?;
        let duration = delay + (plan.seconds + 2.0).min(incoming_available);
        let mix = MixPreview {
            incoming,
            plan,
            delay,
            started: false,
        };
        self.activate_preview(
            PreviewAudio {
                playback,
                file: None,
                mix: Some(mix),
            },
            Some(duration),
        );
        Ok(())
    }

    fn activate_preview(&mut self, audio: PreviewAudio, duration: Option<f64>) {
        let resume_local = self.snapshot.playing;
        self.set_local_playing(false);
        if let Some(playback) = &self.playback {
            self.snapshot.position = playback.position();
        }
        if let Some(preview) = &mut self.preview {
            preview.resume_local = resume_local;
            preview.audio = Some(audio);
        }
        if let Some(preview) = &mut self.snapshot.preview {
            preview.loading = false;
            preview.duration = duration;
        }
        self.change_selection();
        self.apply_preview_volume();
        self.set_preview_playing(true);
    }

    pub(super) fn close_preview(&mut self, resume: bool) {
        let Some(preview) = self.preview.take() else {
            return;
        };
        let resume_local = preview.resume_local;
        let was_active = preview.audio.is_some();
        drop(preview);
        self.snapshot.preview = None;
        if was_active {
            self.change_selection();
        }
        if resume && resume_local {
            self.set_local_playing(true);
        }
    }

    pub(super) fn set_preview_playing(&mut self, playing: bool) {
        let Some(audio) = self
            .preview
            .as_ref()
            .and_then(|preview| preview.audio.as_ref())
        else {
            return;
        };
        if playing {
            audio.playback.player.play();
        } else {
            audio.playback.player.pause();
        }
        if let Some(mix) = &audio.mix {
            if playing && mix.started {
                mix.incoming.player.play();
            } else {
                mix.incoming.player.pause();
            }
        }
        if let Some(preview) = &mut self.snapshot.preview {
            preview.playing = playing;
        }
    }

    pub(super) fn seek_preview(&mut self, position: f64) {
        let Some(preview) = self.active_preview() else {
            return;
        };
        let Some(duration) = preview.duration.filter(|duration| *duration > 0.0) else {
            return;
        };
        if !position.is_finite() {
            return;
        }
        let playing = preview.playing;
        let Some(audio) = self
            .preview
            .as_mut()
            .and_then(|preview| preview.audio.as_ref())
        else {
            return;
        };
        let Some(path) = audio.file.as_ref().map(|file| file.to_path_buf()) else {
            return;
        };
        match self.build_playback(&path, position.clamp(0.0, (duration - 0.01).max(0.0))) {
            Ok((playback, _)) => {
                if let Some(preview) = &mut self.snapshot.preview {
                    preview.position = playback.anchor;
                }
                if let Some(audio) = self
                    .preview
                    .as_mut()
                    .and_then(|preview| preview.audio.as_mut())
                {
                    audio.playback = playback;
                }
                self.apply_preview_volume();
                self.set_preview_playing(playing);
                self.snapshot.error = None;
            }
            Err(error) => self.snapshot.error = Some(error),
        }
    }

    pub(super) fn apply_preview_volume(&self) {
        if let Some(audio) = self
            .preview
            .as_ref()
            .and_then(|preview| preview.audio.as_ref())
        {
            let volume = self.snapshot.volume * 10_f32.powf(-2.67 / 20.0);
            audio.playback.player.set_volume(volume);
            if let Some(mix) = &audio.mix {
                mix.incoming.player.set_volume(volume);
            }
        }
    }

    pub(super) fn tick_preview(&mut self) -> bool {
        let (Some(preview), Some(audio)) = (
            &mut self.snapshot.preview,
            self.preview
                .as_mut()
                .and_then(|preview| preview.audio.as_mut()),
        ) else {
            return false;
        };
        audio.playback.fill();
        if let Some(mix) = &mut audio.mix {
            if !mix.started
                && preview.playing
                && audio.playback.position() - audio.playback.anchor >= mix.delay
            {
                fade_decks(&audio.playback, &mix.incoming, mix.plan);
                mix.started = true;
                mix.incoming.player.play();
            }
            let (position, ended) = if mix.started {
                mix.incoming.fill();
                (
                    mix.delay + mix.incoming.position() - mix.incoming.anchor,
                    mix.incoming.player.empty(),
                )
            } else {
                (
                    audio.playback.position() - audio.playback.anchor,
                    audio.playback.player.empty(),
                )
            };
            preview.position = position;
            if preview.playing
                && (ended
                    || preview
                        .duration
                        .is_some_and(|duration| position >= duration))
            {
                self.close_preview(true);
                return true;
            }
            return false;
        }
        let position = audio.playback.position();
        preview.position = preview
            .duration
            .map_or(position, |duration| position.min(duration));
        if preview.playing && audio.playback.player.empty() {
            self.close_preview(true);
            return true;
        }
        false
    }
}

impl AudioEngine {
    pub(crate) async fn claim_preview(&self, id: u64) -> Result<oneshot::Receiver<()>, StartError> {
        let (reply, response) = oneshot::channel();
        self.0
            .send(Request::ClaimPreview { id, reply })
            .map_err(|_| StartError::Failed)?;
        response
            .await
            .map_err(|_| StartError::Failed)?
            .ok_or(StartError::Cancelled)
    }

    pub(crate) async fn start_preview(&self, id: u64, file: TempPath) -> Result<(), StartError> {
        let (reply, response) = oneshot::channel();
        self.0
            .send(Request::StartPreview { id, file, reply })
            .map_err(|_| StartError::Failed)?;
        response.await.map_err(|_| StartError::Failed)?
    }
}

fn display_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn replacing_preview_cancels_its_download_and_rejects_late_audio() {
        let mut engine = Engine::new(Arc::default());
        engine.prepare_preview("First".into(), String::new());
        let first = engine.snapshot.preview.as_ref().unwrap().id;
        let mut cancelled = engine.claim_preview(first).unwrap();
        assert!(engine.claim_preview(first).is_none());

        engine.prepare_preview("Second".into(), String::new());
        let second = engine.snapshot.preview.as_ref().unwrap().id;
        assert_ne!(first, second);
        assert_eq!(
            cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        );

        let file = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let path = file.to_path_buf();
        assert_eq!(
            engine.start_preview(first, file),
            Err(StartError::Cancelled)
        );
        assert!(!path.exists());
        assert!(engine.claim_preview(second).is_some());
    }

    #[test]
    fn listening_to_a_transition_leaves_the_original_track_at_its_position() {
        use rodio::{buffer::SamplesBuffer, mixer::mixer, ChannelCount, SampleRate};

        let rate = SampleRate::new(8_000).unwrap();
        let channels = ChannelCount::new(2).unwrap();
        let (mixer, mut output) = mixer(channels, rate);
        let deck = |frame: [f32; 2]| {
            Playback::connect(
                &mixer,
                super::super::DecoderReady {
                    decoder: Box::new(SamplesBuffer::new(channels, rate, frame.repeat(16_000))),
                    position: 0.0,
                    duration: Some(2.0),
                },
            )
            .unwrap()
        };
        let mut engine = Engine::new(Arc::default());
        engine.playback = Some(deck([0.25, 0.25]));
        engine.snapshot.path = Some("original".into());
        engine.snapshot.volume = 1.0;
        engine.set_local_playing(true);
        engine.prepare_preview("Transition".into(), String::new());
        let plan = serde_json::from_str::<super::super::PairSettings>(
            r#"{"preset":"linear","seconds":0.5,"trim":0,"cue":0,"intensity":0.7}"#,
        )
        .unwrap()
        .into();
        engine.activate_preview(
            PreviewAudio {
                playback: deck([0.5, 0.0]),
                file: None,
                mix: Some(MixPreview {
                    incoming: deck([0.0, 0.5]),
                    plan,
                    delay: 0.25,
                    started: false,
                }),
            },
            Some(0.95),
        );
        let mut mixed_frame = None;
        for frame in 0..=8_000 {
            if frame % 80 == 0 {
                engine.tick_preview();
            }
            let left = output.next().unwrap();
            let right = output.next().unwrap();
            if frame == 4_000 {
                mixed_frame = Some((left, right));
            }
        }
        let (left, right) = mixed_frame.unwrap();
        assert!((0.1..0.4).contains(&left) && (0.1..0.4).contains(&right));
        assert!(engine.active_preview().is_none());
        assert!(engine.snapshot.playing);
        assert_eq!(engine.snapshot.path.as_deref(), Some("original"));
        assert_eq!(engine.snapshot.position, 0.0);
    }
}
