import { TargetedEvent } from 'preact';
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import { usePlayerStore, useSettingsStore } from '@/stores';
import { type Track } from '@/types/Track';
import { getAudioSrc, getCoverSrc } from '@/utils/audioServer';
import { getPlaybackVolume } from '@/utils/volumeNormalization';

const FALLBACK_COVER = '/temp_cover.png';

type TrackPlayingProps = {
  track: Track | null;
};

const TIME_UPDATE_INTERVAL_MS = 250;
const SEEK_TOLERANCE_SEC = 0.25;
const SEEK_TIMEOUT_MS = 750;

export const TrackPlaying = ({ track }: TrackPlayingProps) => {
  const volume = usePlayerStore((state) => state.volume);
  const playing = usePlayerStore((state) => state.playing);
  const seeking = usePlayerStore((state) => state.seeking);
  const volumeNormEnabled = useSettingsStore((state) => state.volumeNormEnabled);
  const volumeNormLevel = useSettingsStore((state) => state.volumeNormLevel);
  const setDuration = usePlayerStore((state) => state.setDuration);
  const setCurrentTime = usePlayerStore((state) => state.setCurrentTime);
  const setPlaying = usePlayerStore((state) => state.setPlaying);
  const playNext = usePlayerStore((state) => state.playNext);
  const audioRef = useRef<HTMLAudioElement>(null);
  const lastAppliedPlayingRef = useRef<boolean | null>(null);
  const lastTimeUpdateRef = useRef(0);
  const wasSeekingRef = useRef(false);
  const pendingSeekRef = useRef<number | null>(null);
  const suppressPlaybackSyncRef = useRef(false);
  const [playback, setPlayback] = useState<{ path: string; url: string } | null>(null);
  const [coverSrc, setCoverSrc] = useState(FALLBACK_COVER);
  const [ready, setReady] = useState(false);

  const playbackUrl =
    playback && track && playback.path === track.path ? playback.url : undefined;

  const canStartPlayback = Boolean(track && playbackUrl && ready);

  const playbackVolume = getPlaybackVolume(volume, track, volumeNormEnabled, volumeNormLevel);

  const getStoredTime = () => usePlayerStore.getState().currentTime;

  const syncAudioToStoredTime = () => {
    const audio = audioRef.current;
    if (!audio) {
      return false;
    }

    const targetTime = pendingSeekRef.current ?? getStoredTime();
    if (targetTime <= 0) {
      pendingSeekRef.current = null;
      return true;
    }

    if (audio.readyState < HTMLMediaElement.HAVE_METADATA) {
      pendingSeekRef.current = targetTime;
      return false;
    }

    if (Math.abs(audio.currentTime - targetTime) <= SEEK_TOLERANCE_SEC) {
      pendingSeekRef.current = null;
      return true;
    }

    audio.currentTime = targetTime;
    pendingSeekRef.current = null;
    return false;
  };

  const waitForAudioSeek = (targetTime: number) => {
    const audio = audioRef.current;
    if (!audio) {
      return Promise.resolve();
    }

    if (targetTime <= 0 || Math.abs(audio.currentTime - targetTime) <= SEEK_TOLERANCE_SEC) {
      return Promise.resolve();
    }

    if (audio.readyState < HTMLMediaElement.HAVE_METADATA) {
      pendingSeekRef.current = targetTime;
      return Promise.resolve();
    }

    return new Promise<void>((resolve) => {
      let settled = false;

      const finish = () => {
        if (settled) {
          return;
        }

        settled = true;
        audio.removeEventListener('seeked', onSeeked);
        window.clearTimeout(timeoutId);
        resolve();
      };

      const onSeeked = () => finish();
      const timeoutId = window.setTimeout(finish, SEEK_TIMEOUT_MS);

      audio.addEventListener('seeked', onSeeked);
      audio.currentTime = targetTime;
    });
  };

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
    void syncAudioToStoredTime();
  };

  useEffect(() => {
    if (!ready || !playbackUrl || playing || seeking) {
      return;
    }

    void syncAudioToStoredTime();
  }, [ready, playbackUrl, playing, seeking]);

  useLayoutEffect(() => {
    setReady(false);
    wasSeekingRef.current = false;
    pendingSeekRef.current = null;
    lastAppliedPlayingRef.current = null;
  }, [track?.path]);

  useEffect(() => {
    const audio = audioRef.current;

    if (!track) {
      suppressPlaybackSyncRef.current = false;
      setPlayback(null);
      setCurrentTime(0);
      setDuration(0);
      if (audio) {
        audio.pause();
      }
      return;
    }

    const resumeTime = usePlayerStore.getState().currentTime;
    const shouldAutoPlay = usePlayerStore.getState().playing;
    suppressPlaybackSyncRef.current = true;
    setCurrentTime(resumeTime);
    setDuration(track.duration ?? 0);
    pendingSeekRef.current = resumeTime > 0 ? resumeTime : null;

    if (audio) {
      audio.pause();
    }

    if (shouldAutoPlay) {
      setPlaying(true);
    }

    let active = true;

    void getAudioSrc(track.path).then((url) => {
      if (active) {
        setPlayback({ path: track.path, url });
      }
    });

    return () => {
      active = false;
    };
  }, [track?.path]);

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

    audioRef.current.volume = playbackVolume;
  }, [playbackVolume, playbackUrl]);

  useEffect(() => {
    lastAppliedPlayingRef.current = null;
  }, [playbackUrl, ready]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio || !canStartPlayback || lastAppliedPlayingRef.current === playing) {
      return;
    }

    lastAppliedPlayingRef.current = playing;

    if (playing) {
      void waitForAudioSeek(getStoredTime())
        .then(() => audio.play())
        .then(() => {
          suppressPlaybackSyncRef.current = false;
        })
        .catch((error) => {
          suppressPlaybackSyncRef.current = false;
          console.error('Audio play failed', error);
          setPlaying(false);
        });
      return;
    }

    suppressPlaybackSyncRef.current = false;
    audio.pause();
  }, [playing, canStartPlayback, playbackUrl, setPlaying]);

  useEffect(() => {
    const wasSeeking = wasSeekingRef.current;

    if (wasSeeking && !seeking) {
      applySeek(usePlayerStore.getState().currentTime);
    }

    wasSeekingRef.current = seeking;
  }, [seeking]);

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

  const onPlay = () => {
    suppressPlaybackSyncRef.current = false;
    lastAppliedPlayingRef.current = true;
    if (!playing) {
      setPlaying(true);
    }
  };

  const onPause = () => {
    lastAppliedPlayingRef.current = false;
    if (suppressPlaybackSyncRef.current) {
      return;
    }

    if (playing) {
      setPlaying(false);
    }
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
    const audio = e.currentTarget;
    const mediaError = audio.error;

    if (!playbackUrl || !audio.src) {
      return;
    }

    if (mediaError?.code === MediaError.MEDIA_ERR_ABORTED) {
      return;
    }

    console.error('Audio element error', mediaError?.code, mediaError?.message, audio.src);
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
        key={track.path}
        ref={audioRef}
        src={playbackUrl}
        controls={false}
        preload='metadata'
        onLoadedMetadata={onLoadedMetadata}
        onCanPlay={onCanPlay}
        onPlay={onPlay}
        onPause={onPause}
        onTimeUpdate={onTimeUpdate}
        onSeeked={onSeeked}
        onEnded={onEnded}
        onError={onError}
      />
    </div>
  );
};
