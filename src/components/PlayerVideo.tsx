import { useNavigate } from '@tanstack/react-router';
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Progress } from '@/components/Progress';
import { usePlayerStore, useSettingsStore } from '@/stores';
import { refreshPlayerSnapshot } from '@/stores/playerStore';
import { cancelMusicDownload, useMusicDownload } from '@/utils/musicDownload';
import { closePlayerVideo, showPlayerVideo, trackVideoId, usePlayerVideo } from '@/utils/playerVideo';
import { toast } from '@/utils/toast';

export function PlayerVideo({ suspended }: { suspended: boolean }) {
  const request = usePlayerVideo((state) => state.request);
  const path = usePlayerStore((state) => state.currentTrackPath);
  const track = usePlayerStore((state) => state.currentTrack);
  const navigate = useNavigate();
  const progress = useMusicDownload(({ active }) => active?.videoId === request?.id ? active?.progress : undefined);
  const cancelling = useMusicDownload(({ active }) => active?.videoId === request?.id && !!active?.cancelling);
  const video = useRef<HTMLVideoElement>(null);
  const src = request?.src;
  const clipId = usePlayerStore((state) => state.clipId);
  const playing = usePlayerStore((state) => state.playing);
  const [feedback, setFeedback] = useState<{ clipId: number; sequence: number; icon: 'Play' | 'Pause' }>();
  const [pictureLoaded, setPictureLoaded] = useState(false);
  const ready = !!request && clipId === request.key && pictureLoaded;
  const percent = progress?.phase === 'downloading' && !cancelling ? progress.percent : null;

  useEffect(() => {
    if (suspended || !request || request.path === path) return;
    if (path && track?.path !== path) return;
    let active = true;
    const id = trackVideoId(track);
    void Promise.resolve().then(async () => {
      if (!active || usePlayerVideo.getState().request !== request) return;
      if (track && (track.hasLocalVideo || id) && !useMusicDownload.getState().active) {
        await showPlayerVideo(track);
      } else {
        closePlayerVideo();
        if (path) await navigate({ to: '/lyrics', replace: true });
      }
    }).catch(() => {
      if (active) closePlayerVideo();
    });
    return () => { active = false; };
  }, [request, path, track, navigate, suspended]);

  useLayoutEffect(() => {
    const element = video.current;
    if (!element || !request || suspended) return;
    let active = true;
    let playPending = false;
    let playBlocked = false;
    let timeline: number | undefined;
    const sync = () => {
      const state = usePlayerStore.getState();
      if (state.clipId !== request.key || state.currentTrackPath !== request.path
        || (document.hidden && useSettingsStore.getState().suspendHiddenVideo)) {
        element.pause();
        return;
      }
      if (!state.playing) {
        element.pause();
        playBlocked = false;
      }
      const position = Math.min(state.currentTime, element.duration);
      const drift = position - element.currentTime;
      if (!Number.isFinite(drift)) return;
      if (timeline !== state.timeline || Math.abs(drift) > 0.2 || (!state.playing && Math.abs(drift) > 0.03)) {
        element.currentTime = position;
      }
      timeline = state.timeline;
      if (position >= element.duration) {
        element.pause();
      } else if (state.playing && element.paused && !playPending && !playBlocked) {
        playPending = true;
        void element.play().catch((error) => {
          if (!active || (error instanceof DOMException && error.name === 'AbortError')) return;
          playBlocked = true;
          if (!document.hidden && !(error instanceof DOMException && error.name === 'NotAllowedError')) {
            toast.error('Не удалось воспроизвести видео', error);
          }
        }).finally(() => { playPending = false; });
      }
    };
    const loaded = () => {
      setPictureLoaded(true);
      sync();
    };
    const visibilityChanged = () => {
      if (!document.hidden) {
        playBlocked = false;
        void refreshPlayerSnapshot().catch(() => undefined);
      }
      sync();
    };
    const focused = () => {
      playBlocked = false;
      sync();
    };
    setPictureLoaded(element.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA);
    element.addEventListener('loadeddata', loaded);
    document.addEventListener('visibilitychange', visibilityChanged);
    window.addEventListener('focus', focused);
    const unsubscribe = usePlayerStore.subscribe(sync);
    void refreshPlayerSnapshot().catch(() => undefined);
    sync();
    return () => {
      active = false;
      unsubscribe();
      element.removeEventListener('loadeddata', loaded);
      document.removeEventListener('visibilitychange', visibilityChanged);
      window.removeEventListener('focus', focused);
      element.pause();
      element.removeAttribute('src');
      element.load();
    };
  }, [request, path, suspended]);

  if (!request || request.path !== path || suspended) return null;

  const toggleVideo = () => {
    const player = usePlayerStore.getState();
    if (player.clipId !== request.key || player.currentTrackPath !== request.path) return;
    const nextPlaying = !player.playing;
    setFeedback((previous) => ({
      clipId: request.key,
      sequence: (previous?.sequence ?? 0) + 1,
      icon: nextPlaying ? 'Play' : 'Pause',
    }));
    player.setPlaying(nextPlaying);
  };

  return (
    <section className='player-video' aria-label='Видео текущего трека'>
      <div className='player-video-frame'>
        {src && <video key={request.key} ref={video} src={src} muted playsInline preload='auto'
          onError={(event) => {
            if (!event.currentTarget.isConnected || document.hidden || usePlayerVideo.getState().request !== request) return;
            closePlayerVideo();
            toast.error('Не удалось воспроизвести видео');
          }} />}
        {ready && <button type='button' className='player-video-toggle'
          aria-label={playing ? 'Приостановить воспроизведение видео' : 'Продолжить воспроизведение видео'}
          onClick={toggleVideo}>
          {feedback?.clipId === request.key && <span key={feedback.sequence} className='player-video-feedback' aria-hidden='true'
            onAnimationEnd={() => setFeedback((current) => current === feedback ? undefined : current)}>
            <Icon name={feedback.icon} size={40} />
          </span>}
        </button>}
        {!ready && <div className='player-video-loading'>
          {percent === null ? <LoadingIndicator label='Загрузка видео' />
            : <Progress label='Ход загрузки видео' value={percent} max={100} />}
          {!src && progress && progress.phase !== 'importing' &&
            <Button size='small' disabled={cancelling}
              onClick={() => void cancelMusicDownload().catch((error) => toast.error('Не удалось отменить загрузку', error))}>
              Отменить
            </Button>}
        </div>}
      </div>
    </section>
  );
}
