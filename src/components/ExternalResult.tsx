import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';
import type { MusicSearchResult } from '@/utils/ytDlp';

export function ExternalResult({ result, linked, downloading, preview, action }: {
  result: MusicSearchResult;
  linked: boolean;
  downloading: boolean;
  preview: () => void;
  action: () => void;
}) {
  return (
    <article className='search-result'>
      <button type='button' className='search-thumbnail' aria-label={`Слушать «${result.title}»`} onClick={preview}>
        {result.thumbnail && <img src={result.thumbnail} alt='' loading='lazy' />}
        <span className='search-preview-icon'><Icon name='Play' size={42} /></span>
        {result.duration !== null && result.duration > 0 &&
          <span className='search-duration'>{formatPlaybackTime(result.duration)}</span>}
      </button>
      <div className='search-result-body'>
        <div className='track-label'>
          <p className='media-title' title={result.title}>{result.title}</p>
          {result.artist && <p className='track-artist' title={result.artist}>{result.artist}</p>}
          {linked && <p className='track-artist'>В медиатеке</p>}
        </div>
        <Button variant='primary' size='small' disabled={!linked && downloading} onClick={action}
          aria-label={`${linked ? 'Слушать из медиатеки' : 'Загрузить'} «${result.title}»`}>{linked ? 'Слушать' : 'Загрузить'}</Button>
      </div>
    </article>
  );
}
