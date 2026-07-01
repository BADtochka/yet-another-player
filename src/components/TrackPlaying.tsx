import { TargetedEvent } from 'preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import { usePlayerStore } from '@/stores';
import { type Track } from '@/types/Track';
import { getAudioSrc, getCoverSrc } from '@/utils/audioServer';

const FALLBACK_COVER = '/temp_cover.png';

type TrackPlayingProps = {
  track: Track | null;
};

const TIME_UPDATE_INTERVAL_MS = 250;

export const TrackPlaying = ({ track }: TrackPlayingProps) => {
  const volume = usePlayerStore((state) => state.volume);
  const playing = usePlayerStore((state) => state.playing);
  const seeking = usePlayerStore((state) => state.seeking);
  const currentTime = usePlayerStore((state) => state.currentTime);
  const setDuration = usePlayerStore((state) => state.setDuration);
  const setCurrentTime = usePlayerStore((state) => state.setCurrentTime);
  const playNext = usePlayerStore((state) => state.playNext);
  const audioRef = useRef<HTMLAudioElement>(null);
  const lastTimeUpdateRef = useRef(0);
  const wasSeekingRef = useRef(false);
  const pendingSeekRef = useRef<number | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [coverSrc, setCoverSrc] = useState(FALLBACK_COVER);
  const [ready, setReady] = useState(false);

  const applySeek = (time: number) => {
    const audio = audioRef.current;
    if (!audio) {
      return;
    }

    pendingSeekRef.current = time;

    if (audio.readyState >= HTMLMediaElement.HAVE_METADATA) {
      audio.currentTime = time;
      pendingSeekRef.current = null;
    }
  };

  const flushPendingSeek = () => {
    const audio = audioRef.current;
    if (!audio || pendingSeekRef.current == null) {
      return;
    }

    if (audio.readyState >= HTMLMediaElement.HAVE_METADATA) {
      audio.currentTime = pendingSeekRef.current;
      pendingSeekRef.current = null;
    }
  };

  useEffect(() => {
    const audio = audioRef.current;
    setReady(false);
    setSrc(null);
    setCurrentTime(0);
    wasSeekingRef.current = false;
    pendingSeekRef.current = null;

    if (!track) {
      setDuration(0);
      if (audio) {
        audio.pause();
        audio.removeAttribute('src');
        audio.load();
      }
      return;
    }

    setDuration(track.duration ?? 0);

    if (audio) {
      audio.pause();
    }

    let active = true;

    const loadTrack = () => {
      void getAudioSrc(track.path).then((url) => {
        if (active) {
          setSrc(url);
        }
      });
    };

    requestAnimationFrame(loadTrack);

    return () => {
      active = false;
    };
  }, [track?.path, setCurrentTime, setDuration]);

  useEffect(() => {
    if (!track) {
      setCoverSrc(FALLBACK_COVER);
      return;
    }

    let active = true;

    void getCoverSrc(track.path)
      .then((url) => {
        if (active) {
          setCoverSrc(url);
        }
      })
      .catch(() => {
        if (active) {
          setCoverSrc(FALLBACK_COVER);
        }
      });

    return () => {
      active = false;
    };
  }, [track?.path]);

  useEffect(() => {
    if (!audioRef.current) {
      return;
    }

    audioRef.current.volume = volume;
  }, [volume, src]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio || !ready) {
      wasSeekingRef.current = seeking;
      return;
    }

    if (playing) {
      requestAnimationFrame(() => {
        void audio.play().catch((error) => {
          console.error('Audio play failed', error);
        });
      });
    } else {
      audio.pause();
    }
  }, [playing, ready, src]);

  useEffect(() => {
    const wasSeeking = wasSeekingRef.current;

    if (wasSeeking && !seeking) {
      applySeek(currentTime);
    }

    wasSeekingRef.current = seeking;
  }, [currentTime, seeking]);

  const onLoadedMetadata = (e: TargetedEvent<HTMLAudioElement>) => {
    setReady(true);

    if (Number.isFinite(e.currentTarget.duration) && e.currentTarget.duration > 0) {
      setDuration(e.currentTarget.duration);
    }

    flushPendingSeek();
  };

  const onCanPlay = (e: TargetedEvent<HTMLAudioElement>) => {
    if (Number.isFinite(e.currentTarget.duration) && e.currentTarget.duration > 0) {
      setDuration(e.currentTarget.duration);
    }

    flushPendingSeek();
  };

  const onTimeUpdate = (e: TargetedEvent<HTMLAudioElement>) => {
    if (seeking) {
      return;
    }

    const now = performance.now();
    if (now - lastTimeUpdateRef.current < TIME_UPDATE_INTERVAL_MS) {
      return;
    }

    lastTimeUpdateRef.current = now;
    setCurrentTime(e.currentTarget.currentTime);
  };

  const onSeeked = (e: TargetedEvent<HTMLAudioElement>) => {
    setCurrentTime(e.currentTarget.currentTime);
    lastTimeUpdateRef.current = performance.now();
  };

  const onEnded = () => {
    lastTimeUpdateRef.current = 0;
    playNext();
  };

  const onError = (e: TargetedEvent<HTMLAudioElement>) => {
    const mediaError = e.currentTarget.error;
    console.error('Audio element error', mediaError?.code, mediaError?.message);
    setReady(false);
  };

  if (!track) return null;

  const artistLabel = track.artists.length > 0 ? track.artists.join(', ') : 'Unknown artist';

  return (
    <div className='flex items-center gap-4 absolute left-4'>
      <img
        className='size-16 rounded-lg border border-[#21366C] object-cover'
        src={coverSrc}
        alt=''
        onError={(event) => {
          const image = event.currentTarget;

          if (!image.src.includes('temp_cover.png')) {
            image.src = FALLBACK_COVER;
          }
        }}
      />
      <div>
        <p>{track.name}</p>
        <p>{artistLabel}</p>
      </div>
      <audio
        ref={audioRef}
        src={src ?? undefined}
        crossOrigin='anonymous'
        controls={false}
        preload='metadata'
        onLoadedMetadata={onLoadedMetadata}
        onCanPlay={onCanPlay}
        onTimeUpdate={onTimeUpdate}
        onSeeked={onSeeked}
        onEnded={onEnded}
        onError={onError}
      />
    </div>
  );
};
