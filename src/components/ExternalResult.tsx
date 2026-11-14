import { useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { Spinner } from '@/components/LoadingIndicator';
import { Progress } from '@/components/Progress';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';
import { cancelMusicDownload, useMusicDownload } from '@/utils/musicDownload';
import { useVideoAvailable } from '@/utils/playerVideo';
import { toast } from '@/utils/toast';
import type { MusicSearchResult } from '@/utils/ytDlp';

export function ExternalResult({ result, localPath, disabled, action, video }: {
  result: MusicSearchResult;
  localPath?: string;
  disabled: boolean;
  action: () => void;
  video: () => void;
}) {
  const [inspected, setInspected] = useState(false);
  const available = useVideoAvailable(inspected && result.videoAvailable == null ? result.id : undefined);
  const showVideo = result.videoAvailable ?? available;
  const download = useMusicDownload(({ active }) => active?.videoId === result.id ? active : null);
  const progress = download?.progress;
  const percent = progress?.phase === 'downloading' && !download?.cancelling ? progress.percent : null;
  const canCancel = !!progress && progress.phase !== 'importing';

  return (
    <article className='search-result' data-track-path={localPath} data-external-video={result.id}
      onPointerEnter={() => setInspected(true)} onFocusIn={() => setInspected(true)}>
      <button type='button' className='search-result-action'
        aria-label={`Слушать «${result.title}»`} aria-busy={!!download} disabled={disabled || !!download} onClick={action}>
        <span className='search-thumbnail'>
          {result.thumbnail && <img src={result.thumbnail} alt='' loading='lazy' />}
          <span className='search-play-icon' aria-hidden='true'>
            {download ? percent === null && <Spinner size={28} /> : <Icon name='Play' size={42} />}
          </span>
          {result.duration !== null && result.duration > 0 &&
            <span className='search-duration'>{formatPlaybackTime(result.duration)}</span>}
        </span>
        <span className='search-result-body track-label'>
          <span className='media-title' title={result.title}>{result.title}</span>
          <span className='track-artist' title={result.artist ?? undefined}>{result.artist ?? 'Автор не указан'}</span>
          {localPath && <span className='track-artist'>В медиатеке</span>}
        </span>
      </button>
      {!download && showVideo && <Button className='search-card-video' size='icon' icon='Video'
        title='Смотреть видео' aria-label={`Смотреть «${result.title}»`} disabled={disabled} onClick={video} />}
      {percent !== null && <Progress className='search-card-progress' max={100} value={percent} label='Ход загрузки' />}
      {canCancel && <Button className='search-card-cancel' size='small' variant='secondary'
        disabled={download.cancelling} aria-label='Отменить загрузку'
        onClick={() => void cancelMusicDownload().catch((error) => toast.error('Не удалось отменить загрузку', error))}>
        Отменить
      </Button>}
    </article>
  );
}
