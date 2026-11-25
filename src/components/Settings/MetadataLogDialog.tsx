import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'preact/hooks';
import { Icon } from '@/components/Icons/Icon';
import { metadataSources } from '@/components/Settings/ContentSourcesSettings';
import type { MetadataSource } from '@/types/Settings';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Progress } from '@/components/Progress';
import { TrackAlbumLink, TrackArtistLinks } from '@/components/TrackLinks';
import { useMetadataLookup } from '@/hooks/useLibrary';
import type { Track } from '@/types/Track';
import { describePlayback } from '@/utils/trackMetadata';

const statuses = {
  updated: 'Обновлено',
  unchanged: 'Изменений нет',
  missing: 'Нет совпадения',
  unavailable: 'Файл недоступен',
  searching: 'Поиск',
  error: 'Ошибка поиска',
};
const attemptStatuses = { found: 'Найдено', missing: 'Нет совпадения', searching: 'Поиск', error: 'Ошибка ответа', cancelled: 'Сервис отключён' };
const catalogs: Record<string, string> = {
  applemusic: 'Apple Music', apple_music: 'Apple Music', deezer: 'Deezer',
  musicbrainz: 'MusicBrainz', music_brainz: 'MusicBrainz',
  ytmusic: 'YouTube Music', youtube_music: 'YouTube Music',
};
type Attempt = { source: MetadataSource; status: keyof typeof attemptStatuses; cached: boolean; catalogs: string[] };
type Entry = { track: Track; status: keyof typeof statuses; attempts: Attempt[] };
type LogPage = { entries: Entry[]; scanned: number; total: number; updated: number; issues: number };

const sourceName = (source: MetadataSource) => metadataSources.find(({ value }) => value === source)?.label ?? source;

const readPage = (folder: string, before?: number) => invoke<LogPage>('library_metadata_log', { folder, before });

export function MetadataLogDialog({ folder, onDismiss }: { folder: string; onDismiss: () => void }) {
  const [page, setPage] = useState<LogPage>();
  const [selected, setSelected] = useState<number>();
  const [error, setError] = useState<string>();
  const [paging, setPaging] = useState(false);
  const running = useMetadataLookup((state) => state.running);

  useEffect(() => {
    let active = true;
    let request = 0;
    const refresh = () => {
      const revision = ++request;
      void readPage(folder).then((latest) => {
        if (!active || revision !== request) return;
        const firstPage = latest.entries;
        const oldest = firstPage[firstPage.length - 1]?.track.id;
        setPage((previous) => ({
          ...latest,
          entries: [...firstPage, ...(oldest === undefined || !previous || latest.scanned < previous.scanned
            ? [] : previous.entries.filter(({ track }) => track.id < oldest))],
        }));
        setError(undefined);
      }).catch((reason) => {
        if (active && revision === request) setError(String(reason));
      });
    };
    const listener = listen('library:metadata-log', refresh);
    void listener.then(refresh, refresh);
    return () => {
      active = false;
      void listener.then((unlisten) => unlisten(), () => undefined);
    };
  }, [folder]);

  const loadMore = async () => {
    if (!page || paging) return;
    const oldest = page.entries[page.entries.length - 1]?.track.id;
    if (oldest === undefined) return;
    setPaging(true);
    try {
      const next = await readPage(folder, oldest);
      setPage((current) => !current || current.scanned < page.scanned ? current : {
        ...next,
        entries: [...current.entries, ...next.entries.filter(({ track }) =>
          track.id < (current.entries[current.entries.length - 1]?.track.id ?? Infinity))],
      });
      setError(undefined);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setPaging(false);
    }
  };

  return <Dialog open className='metadata-log-dialog' title='Журнал поиска метаданных'
    description={<span className='metadata-log-folder'>{folder}</span>} onDismiss={onDismiss}>
    {error && <p className='kit-error' role='alert'>{error}</p>}
    {!page && !error && <LoadingIndicator label='Загрузка журнала' />}
    {page && page.total > 0 && <div className='metadata-log-summary'>
      <div className='metadata-log-stats'>
        <div><span>Проверено</span><strong>{page.scanned} из {page.total}</strong></div>
        <div><span>Обновлено</span><strong>{page.updated}</strong></div>
        <div><span>Ошибки и недоступные файлы</span><strong>{page.issues}</strong></div>
      </div>
      <Progress label='Проверено треков' value={page.scanned} max={page.total} />
    </div>}
    {running && <LoadingIndicator label='Поиск метаданных' />}
    {page && !page.entries.length && !running && <p className='kit-message'>
      {page.total ? 'Для этой папки пока нет записей.' : 'В папке пока нет треков медиатеки.'}
    </p>}
    {page && page.entries.length > 0 && <div className='metadata-log-list'>
      <div className='metadata-log-columns' aria-hidden='true'><span>Трек</span><span>Сервисы</span><span>Результат</span></div>
      <ul>
        {page.entries.map(({ track, status, attempts }) => {
          const expanded = selected === track.id;
          const interrupted = status === 'searching' && !running;
          return <li key={track.id}>
            <button type='button' className='metadata-log-row' aria-expanded={expanded}
              onClick={() => setSelected((id) => id === track.id ? undefined : track.id)}>
              <span className='metadata-log-track'><Icon name='Track' size={16} />
                <span><strong>{track.name}</strong><small>{track.artists.join(', ')}</small></span>
              </span>
              <span className='metadata-log-sources'>
                {attempts.length ? attempts.map((attempt) => <span key={attempt.source} data-state={attempt.status}
                  className='metadata-log-source'>{sourceName(attempt.source)}</span>) : '—'}
              </span>
              <span className='metadata-log-result' data-state={interrupted ? 'interrupted' : status}>
                {interrupted ? 'Поиск прерван' : statuses[status]}
              </span>
            </button>
            {expanded && <div className='metadata-log-details'>
              {!!attempts.length && <ol className='metadata-log-attempts' aria-label='Порядок запросов'>
                {attempts.map(({ source, status: attempt, cached, catalogs: providers }) => <li key={source}>
                  <strong>{sourceName(source)}</strong>
                  <span>{attempt === 'searching' && interrupted ? 'Поиск прерван' : attemptStatuses[attempt]}
                    {cached ? ' · Из кэша' : ''}</span>
                  {!!providers.length && <small>{providers.map((provider) => catalogs[provider] ?? provider).join(', ')}</small>}
                </li>)}
              </ol>}
              <dl className='track-file-details'>
                <dt>Исполнители</dt><dd><TrackArtistLinks artists={track.artists} /></dd>
                <dt>Альбом</dt><dd>{track.album ? <TrackAlbumLink track={track}>{track.album}</TrackAlbumLink> : '—'}</dd>
                {describePlayback([track]).map(({ icon, label }) => <div className='contents' key={icon}>
                  <dt>{icon === 'Calendar' ? 'Дата' : icon === 'Time' ? 'Прослушивания' : 'Жанры'}</dt><dd>{label}</dd>
                </div>)}
                <dt>Файл</dt><dd>{track.path}</dd>
              </dl>
            </div>}
          </li>;
        })}
      </ul>
      {page.entries.length < page.scanned + page.entries.filter(({ status }) => status === 'searching').length
        && <Button className='metadata-log-more' size='small' pending={paging} onClick={() => void loadMore()}>Показать ещё</Button>}
    </div>}
  </Dialog>;
}
