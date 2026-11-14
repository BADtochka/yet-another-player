import { useNavigate } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Progress } from '@/components/Progress';
import { usePlayerStore } from '@/stores';
import { cancelMusicDownload, useMusicDownload } from '@/utils/musicDownload';
import { checkPlayerVideo, closePlayerVideo, showPlayerVideo, trackVideoId, usePlayerVideo } from '@/utils/playerVideo';
import { toast } from '@/utils/toast';

export function PlayerVideo() {
  const request = usePlayerVideo((state) => state.request);
  const path = usePlayerStore((state) => state.currentTrackPath);
  const track = usePlayerStore((state) => state.currentTrack);
  const navigate = useNavigate();
  const progress = useMusicDownload(({ active }) => active?.videoId === request?.id ? active?.progress : undefined);
  const cancelling = useMusicDownload(({ active }) => active?.videoId === request?.id && !!active?.cancelling);
  const video = useRef<HTMLVideoElement>(null);
  const [readySrc, setReadySrc] = useState<string>();
  const src = request?.src;
  const ready = !!src && readySrc === src;
  const percent = progress?.phase === 'downloading' && !cancelling ? progress.percent : null;

  useEffect(() => {
    if (!request || request.path === path) return;
    if (path && track?.path !== path) return;
    let active = true;
    const id = trackVideoId(track);
    const available = track?.hasLocalVideo ? Promise.resolve(true) : id ? checkPlayerVideo(id) : Promise.resolve(false);
    void available.then(async (hasVideo) => {
      if (!active || usePlayerVideo.getState().request !== request) return;
      if (hasVideo && track && !useMusicDownload.getState().active) {
        await showPlayerVideo(track);
      } else {
        closePlayerVideo();
        if (path) await navigate({ to: '/lyrics', replace: true });
      }
    }).catch(() => {
      if (active) closePlayerVideo();
    });
    return () => { active = false; };
  }, [request, path, track, navigate]);

  useEffect(() => {
    const element = video.current;
    if (!element || !ready) return;
    const sync = (state: ReturnType<typeof usePlayerStore.getState>) => {
      if (state.currentTrackPath !== request?.path) {
        element.pause();
        return;
      }
      const position = Math.min(state.currentTime, element.duration);
      if (Number.isFinite(position) && Math.abs(element.currentTime - position) > 0.3) {
        element.currentTime = position;
      }
      if (!state.playing && !element.paused) element.pause();
      else if (state.playing && element.paused && position < element.duration) {
        void element.play().catch((error) => {
          if (error instanceof DOMException && error.name === 'AbortError') return;
          if (video.current === element) {
            closePlayerVideo();
            toast.error('Не удалось воспроизвести видео', error);
          }
        });
      }
    };
    element.currentTime = Math.min(usePlayerStore.getState().currentTime, element.duration);
    sync(usePlayerStore.getState());
    const unsubscribe = usePlayerStore.subscribe(sync);
    return () => { unsubscribe(); element.pause(); };
  }, [request, ready]);

  if (!request || request.path !== path) return null;
  return (
    <section className='player-video' aria-label='Видео текущего трека'>
      <div className='player-video-frame'>
        {src && <video key={src} ref={video} src={src} muted playsInline
          onLoadedData={() => setReadySrc(src)}
          onError={() => { closePlayerVideo(); toast.error('Не удалось воспроизвести видео'); }} />}
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
