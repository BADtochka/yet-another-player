use std::{io, path::Path, sync::Arc, time::Duration};

use rodio::{source::SeekError, ChannelCount, Sample, SampleRate, Source};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL},
    errors::Error,
    formats::{FormatReader, SeekMode, SeekTo},
    units::TimeBase,
};

use super::{probe_format, AudioError};

pub(super) struct Mp4Audio {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    time_base: TimeBase,
    duration: Option<Duration>,
    rate: SampleRate,
    channels: ChannelCount,
    samples: Option<SampleBuffer<Sample>>,
    cursor: usize,
    ended: bool,
}

impl Mp4Audio {
    pub(super) fn open(path: &Path) -> Result<Self, AudioError> {
        let reader = probe_format(path)?;
        // The default MP4 stream can be video. Select the audio stream for both duration and seek.
        let (track, decoder) = reader
            .tracks()
            .iter()
            .filter(|track| {
                track.codec_params.codec != CODEC_TYPE_NULL
                    && track.codec_params.sample_rate.is_some()
            })
            .find_map(|track| {
                symphonia::default::get_codecs()
                    .make(&track.codec_params, &DecoderOptions::default())
                    .ok()
                    .map(|decoder| (track, decoder))
            })
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "No supported MP4 audio track")
            })?;
        let params = &track.codec_params;
        let track_id = track.id;
        let time_base = params.time_base.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "MP4 audio time base is missing")
        })?;
        let duration = params
            .n_frames
            .map(|frames| time_base.calc_time(frames).into());
        let mut source = Self {
            reader,
            decoder,
            track_id,
            time_base,
            duration,
            rate: SampleRate::MIN,
            channels: ChannelCount::MIN,
            samples: None,
            cursor: 0,
            ended: false,
        };
        if !source.read_packet(None)? {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "MP4 audio track is empty").into(),
            );
        }
        Ok(source)
    }

    fn read_packet(&mut self, target: Option<u64>) -> Result<bool, AudioError> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(false)
                }
                Err(error) => return Err(error.into()),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let audio = match self.decoder.decode(&packet) {
                Ok(audio) => audio,
                Err(Error::DecodeError(_)) => continue,
                Err(error) => return Err(error.into()),
            };
            let spec = *audio.spec();
            let channels = spec.channels.count();
            let skip = target.map_or(0, |time| {
                let offset: Duration = self
                    .time_base
                    .calc_time(time.saturating_sub(packet.ts()))
                    .into();
                (offset.as_secs_f64() * f64::from(spec.rate)).round() as usize
            });
            if skip >= audio.frames() {
                continue;
            }
            self.rate = SampleRate::new(spec.rate).ok_or("Invalid MP4 sample rate")?;
            self.channels =
                ChannelCount::new(u16::try_from(channels)?).ok_or("Invalid MP4 channel count")?;
            let capacity = audio.capacity() * channels;
            if self
                .samples
                .as_ref()
                .is_some_and(|samples| samples.capacity() < capacity)
            {
                self.samples = None;
            }
            self.samples
                .get_or_insert_with(|| SampleBuffer::new(audio.capacity() as u64, spec))
                .copy_interleaved_ref(audio);
            self.cursor = skip * channels;
            return Ok(true);
        }
    }
}

impl Iterator for Mp4Audio {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.ended {
            return None;
        }
        if self.cursor >= self.samples.as_ref()?.len() {
            match self.read_packet(None) {
                Ok(true) => {}
                Ok(false) => self.ended = true,
                Err(error) => {
                    eprintln!("MP4 audio decoding failed: {error}");
                    self.ended = true;
                }
            }
            if self.ended {
                return None;
            }
        }
        let sample = self.samples.as_ref()?.samples()[self.cursor];
        self.cursor += 1;
        Some(sample)
    }
}

impl Source for Mp4Audio {
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
        self.duration
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let preroll = position.saturating_sub(Duration::from_millis(80));
        self.reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: preroll.into(),
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|error| SeekError::Other(Arc::new(error)))?;
        self.decoder.reset();
        let target = self.time_base.calc_timestamp(position.into());
        self.ended = !self
            .read_packet(Some(target))
            .map_err(|error| SeekError::Other(error.into()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeks_using_the_aac_timeline_when_the_first_stream_is_video() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library/fixtures/video-with-aac.mp4");
        let mut source = Mp4Audio::open(&path).unwrap();
        assert!((2.0..2.1).contains(&source.total_duration().unwrap().as_secs_f64()));
        let whole_clip = Mp4Audio::open(&path).unwrap().count();
        source.try_seek(Duration::from_millis(1500)).unwrap();
        assert_eq!(source.count(), whole_clip - 24_000);
    }
}
