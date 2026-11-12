use std::{fs::File, io, path::Path, sync::Arc, time::Duration};

use opus_decoder::{OpusDecoder, OpusMultistreamDecoder};
use rodio::{source::SeekError, ChannelCount, Decoder, Sample, SampleRate, Source};
use symphonia::core::{
    codecs::CODEC_TYPE_OPUS,
    errors::Error as MediaError,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::Hint,
    units::TimeBase,
};

type AudioError = Box<dyn std::error::Error + Send + Sync>;

pub fn open(path: &Path) -> Result<Box<dyn Source + Send>, AudioError> {
    match Decoder::try_from(File::open(path)?) {
        Ok(decoder) => Ok(Box::new(decoder)),
        Err(original) => match OpusSource::open(path)? {
            Some(decoder) => Ok(Box::new(decoder)),
            None => Err(original.into()),
        },
    }
}

enum Opus {
    Standard(Box<OpusDecoder>),
    Multistream(OpusMultistreamDecoder),
}

struct OpusSource {
    reader: Box<dyn FormatReader>,
    decoder: Opus,
    track_id: u32,
    time_base: TimeBase,
    channels: ChannelCount,
    duration: Option<Duration>,
    initial_skip: usize,
    pre_skip: usize,
    gain: f32,
    pcm: Vec<Sample>,
    cursor: usize,
    end: usize,
    ended: bool,
}

const RATE: u32 = 48_000;
const MAX_FRAMES: usize = 5_760;

impl OpusSource {
    fn open(path: &Path) -> Result<Option<Self>, AudioError> {
        let input = MediaSourceStream::new(
            Box::new(File::open(path)?),
            MediaSourceStreamOptions::default(),
        );
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            hint.with_extension(extension);
        }
        let options = FormatOptions {
            enable_gapless: true,
            ..FormatOptions::default()
        };
        let reader = symphonia::default::get_probe()
            .format(&hint, input, &options, &MetadataOptions::default())?
            .format;
        let Some(track) = reader
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec == CODEC_TYPE_OPUS)
        else {
            return Ok(None);
        };

        let params = &track.codec_params;
        let header = params
            .extra_data
            .as_deref()
            .filter(|header| header.starts_with(b"OpusHead") && header.len() >= 19)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid Opus header"))?;
        let channels = ChannelCount::new(u16::from(header[9]))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing Opus channels"))?;
        let count = usize::from(channels.get());
        let decoder = if header[18] == 0 {
            Opus::Standard(Box::new(OpusDecoder::new(RATE, count)?))
        } else {
            let mapping = header.get(21..21 + count).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Missing Opus channel mapping")
            })?;
            Opus::Multistream(OpusMultistreamDecoder::new(
                RATE,
                count,
                usize::from(header[19]),
                usize::from(header[20]),
                mapping,
            )?)
        };
        let time_base = params.time_base.unwrap_or(TimeBase::new(1, RATE));
        let duration = params
            .n_frames
            .map(|frames| time_base.calc_time(frames).into());
        let pre_skip = usize::from(u16::from_le_bytes([header[10], header[11]]));
        let gain =
            10_f32.powf(f32::from(i16::from_le_bytes([header[16], header[17]])) / (256.0 * 20.0));

        Ok(Some(Self {
            track_id: track.id,
            reader,
            decoder,
            time_base,
            channels,
            duration,
            initial_skip: pre_skip,
            pre_skip,
            gain,
            pcm: vec![0.0; MAX_FRAMES * count],
            cursor: 0,
            end: 0,
            ended: false,
        }))
    }

    fn read_packet(&mut self, seek: Option<Duration>) -> Result<bool, AudioError> {
        let count = usize::from(self.channels.get());
        loop {
            let packet = match self.reader.next_packet() {
                Ok(packet) => packet,
                Err(MediaError::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(false)
                }
                Err(error) => return Err(error.into()),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let frames = match &mut self.decoder {
                Opus::Standard(decoder) => {
                    decoder.decode_float(&packet.data, &mut self.pcm, false)?
                }
                Opus::Multistream(decoder) => {
                    decoder.decode_float(&packet.data, &mut self.pcm, false)?
                }
            };
            let skip = self.pre_skip.min(frames);
            self.pre_skip -= skip;
            let trim = (packet.trim_start() as usize).max(skip);
            let end = frames.saturating_sub(packet.trim_end() as usize);
            let offset = seek.map_or(0, |target| {
                let start: Duration = self.time_base.calc_time(packet.ts()).into();
                (target.saturating_sub(start).as_secs_f64() * f64::from(RATE)).round() as usize
            });
            self.cursor = trim.saturating_add(offset).min(end) * count;
            self.end = end * count;
            if self.cursor < self.end {
                return Ok(true);
            }
        }
    }
}

impl Iterator for OpusSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.ended {
            return None;
        }
        if self.cursor == self.end {
            match self.read_packet(None) {
                Ok(true) => {}
                Ok(false) => {
                    self.ended = true;
                    return None;
                }
                Err(error) => {
                    eprintln!("Opus decoding failed: {error}");
                    self.ended = true;
                    return None;
                }
            }
        }
        let sample = self.pcm[self.cursor] * self.gain;
        self.cursor += 1;
        Some(sample)
    }
}

impl Source for OpusSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(RATE).expect("nonzero sample rate")
    }
    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let preroll = position.saturating_sub(Duration::from_millis(80));
        let target = SeekTo::Time {
            time: preroll.into(),
            track_id: Some(self.track_id),
        };
        let seeked = self
            .reader
            .seek(SeekMode::Accurate, target)
            .map_err(|error| SeekError::Other(Arc::new(error)))?;
        match &mut self.decoder {
            Opus::Standard(decoder) => decoder.reset(),
            Opus::Multistream(decoder) => decoder.reset(),
        }
        self.pre_skip = if seeked.actual_ts == 0 {
            self.initial_skip
        } else {
            0
        };
        self.cursor = 0;
        self.end = 0;
        self.ended = !self
            .read_packet(Some(position))
            .map_err(|error| SeekError::Other(error.into()))?;
        Ok(())
    }
}
