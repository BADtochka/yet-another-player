import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { ExternalResult } from '@/components/ExternalResult';
import { Progress } from '@/components/Progress';
import { CollectionCard, CollectionList } from '@/components/Collections';
import { SeekBar } from '@/components/SeekBar';
import { Track } from '@/components/Track';
import { YtDlpChecking } from '@/components/YtDlpFeedback';
import { usePlayerStore, useSettingsStore } from '@/stores';
import { useCollections, useTrackPages, useYoutubeTracks, type TrackQuery } from '@/hooks/useLibrary';
import { closePreview, playTrackQuery, preparePreview, seekPreview, setPreviewPlaying, type AudioPreview } from '@/stores/playerStore';
import { addSearchHistory } from '@/stores/searchHistory';
import type { Track as LibraryTrack } from '@/types/Track';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';
import { cancelMusicDownload, downloadMusic, useMusicDownload } from '@/utils/musicDownload';
import { toast } from '@/utils/toast';
import { runYtDlpSearch, startMusicPreview, useYtDlpStatus, YtDlpError, type MusicSearchResult } from '@/utils/ytDlp';

const tabs = [
  ['all', 'Все'], ['tracks', 'Треки'], ['playlists', 'Плейлисты'],
  ['artists', 'Исполнители'], ['albums', 'Альбомы'],
] as const;
type Tab = typeof tabs[number][0];

export const Route = createFileRoute('/search')({
  validateSearch: (search: Record<string, unknown>) => ({
    q: typeof search.q === 'string' ? search.q : '',
    tab: tabs.some(([value]) => value === search.tab) ? search.tab as Tab : undefined,
    request: typeof search.request === 'number' && Number.isSafeInteger(search.request) && search.request > 0 ? search.request : undefined,
  }),
  component: Search,
});

function Search() {
  const { q, tab = 'all', request } = Route.useSearch();
  const navigate = Route.useNavigate();
  const [remote, setRemote] = useState<{ key: string; items: MusicSearchResult[]; error?: string }>();
  const [searchAttempt, setSearchAttempt] = useState(0);
  const checkingTool = useYtDlpStatus((state) => state.checking);
  const preview = usePlayerStore((state) => state.preview);
  const previewRequest = useRef<{ id?: number }>();
  const downloading = useMusicDownload((state) => state.active !== null);
  const query = q.trim();
  const remoteVisible = query.length >= 2;
  const remoteKey = JSON.stringify([query, request, searchAttempt]);
  const currentRemote = remote?.key === remoteKey ? remote : undefined;
  const external = currentRemote?.items ?? [];
  const links = useYoutubeTracks(external.map(({ id }) => id));
  const remoteError = currentRemote?.error;
  const searching = remoteVisible && !currentRemote;
  const visible = (kind: Tab) => query !== '' && (tab === 'all' || tab === kind);
  const trackQuery: TrackQuery = { text: query, sort: 'path' };
  const tracks = useTrackPages(trackQuery, visible('tracks'));
  const albums = useCollections('album', query, visible('albums'));
  const artists = useCollections('artist', query, visible('artists'));
  const playlists = useCollections('playlist', query, visible('playlists'));

  useEffect(() => {
    if (!remoteVisible) return;
    let active = true;
    void runYtDlpSearch(query).then((items) => {
      if (active) setRemote({ key: remoteKey, items });
    }).catch((error) => {
      if (active) {
        setRemote({ key: remoteKey, items: [], error: error instanceof YtDlpError ? error.message : 'Внешний поиск недоступен.' });
      }
    });
    return () => {
      active = false;
      const id = previewRequest.current?.id;
      previewRequest.current = undefined;
      if (id !== undefined) closePreview(id);
    };
  }, [remoteKey, remoteVisible]);

  const indexedIds = new Set(tracks.tracks.map((track) => track.id));
  const linkedTracks = new Map(Object.values(links?.data ?? {}).map((track) => [track.id, track]));
  const localTracks = visible('tracks')
    ? [...tracks.tracks, ...Array.from(linkedTracks.values()).filter((track) => !indexedIds.has(track.id))]
    : [];
  const localFound = !!(localTracks.length || albums?.data?.length || artists?.data?.length || playlists?.data?.length);
  const localLoading = tracks.loading || (visible('tracks') && external.length > 0 && !links)
    || (visible('albums') && !albums) || (visible('artists') && !artists)
    || (visible('playlists') && !playlists);
  const localError = tracks.error ?? albums?.error ?? artists?.error ?? playlists?.error;

  const onPreview = async (result: MusicSearchResult) => {
    if (previewRequest.current && previewRequest.current.id === undefined) return;
    const request: { id?: number } = {};
    previewRequest.current = request;
    try {
      const id = await preparePreview(result.title, result.artist ?? '');
      request.id = id;
      if (previewRequest.current !== request) {
        closePreview(id);
        return;
      }
      await startMusicPreview(id, result.id);
    } catch (error) {
      if (previewRequest.current !== request) return;
      previewRequest.current = undefined;
      const cancelled = error instanceof YtDlpError && error.state === 'previewCancelled';
      if (!cancelled && !isGatekeeperBlock(error)) {
        toast.error('Не удалось открыть предпросмотр', error);
      }
    }
  };

  const onDownload = async (result: MusicSearchResult) => {
    if (downloading) return;
    if (useSettingsStore.getState().musicFolders.length === 0) {
      toast.error('Добавьте папку медиатеки в настройках');
      return;
    }
    try {
      const { alreadyPresent } = await downloadMusic(result.id, 0);
      toast.info(alreadyPresent ? 'Трек уже в медиатеке' : 'Трек добавлен в медиатеку');
    } catch (error) {
      if (error instanceof YtDlpError && error.state === 'downloadCancelled') {
        toast.info(error.message);
      } else if (!isGatekeeperBlock(error)) {
        toast.error('Не удалось добавить трек', error);
      }
    }
  };

  const playLocal = (track: LibraryTrack) => {
    addSearchHistory({ query, trackId: track.id, title: track.name, artist: track.artists.join(', ') });
    if (indexedIds.has(track.id)) playTrackQuery(trackQuery, { type: 'id', id: track.id });
    else usePlayerStore.getState().playTrack(track, [track]);
  };

  const collections = [
    { kind: 'playlist', items: playlists?.data ?? [], route: '/playlists', tab: 'playlists' },
    { kind: 'artist', items: artists?.data ?? [], route: '/artists', tab: 'artists' },
    { kind: 'album', items: albums?.data ?? [], route: '/', tab: 'albums' },
  ] as const;
  const localTitle = {
    all: 'Локальная медиатека', tracks: 'Локальные треки', playlists: 'Локальные плейлисты',
    artists: 'Локальные исполнители', albums: 'Локальные альбомы',
  }[tab];
  const noRemote = remoteVisible && !searching && !remoteError && external.length === 0;
  const noLocal = !localFound && !localLoading && !localError;
  const noResults = noLocal && (noRemote || !remoteVisible);
  const focusSearch = () => {
    const input = document.querySelector<HTMLInputElement>('.app-search input');
    input?.focus();
    input?.select();
  };

  if (!query) return <DownloadStatus />;

  return (
    <section className='search-page'>
      <header className='search-heading'>
        <h1>{noResults ? 'Поиск' : `Результаты для «${query}»`}</h1>
        <p>{noResults ? 'Проверьте написание или измените запрос' : 'Сначала внешние результаты, затем локальная медиатека'}</p>
      </header>
      <div className='search-tabs' role='group' aria-label='Категория результатов'>
        {tabs.map(([value, title]) => (
          <Button key={value} variant={tab === value ? 'primary' : 'secondary'} size='small' aria-pressed={tab === value}
            onClick={() => void navigate({ search: { q, tab: value, request } })}>{title}</Button>
        ))}
      </div>
      <DownloadStatus />
      {remoteVisible && (
        <section className='search-section search-remote'>
          {noRemote ? (
            <div className='search-empty'>
              <p>Внешний поиск</p>
              <h2>Ничего не найдено</h2>
              <p>Попробуйте более короткий запрос, имя автора или прямую ссылку.</p>
              <Button size='small' variant='primary' onClick={focusSearch}>Изменить запрос</Button>
            </div>
          ) : (
            <header className='search-heading'>
              <h2>Внешние результаты</h2>
              <p>Песни из YouTube Music и музыкальные видео</p>
            </header>
          )}
          {searching && <YtDlpChecking label={checkingTool ? undefined : 'Ищем в YouTube Music…'} />}
          {remoteError && (
            <div className='search-empty' role='status'>
              <h3>Внешний поиск недоступен</h3>
              <p>{remoteError}</p>
              <Button size='small' onClick={() => setSearchAttempt((attempt) => attempt + 1)}>Повторить поиск</Button>
            </div>
          )}
          {preview && <PreviewControls key={preview.id} preview={preview} />}
          {external.length > 0 && (
            <div className='search-external'>
              {external.map((result) => {
                const local = links?.data?.[result.id];
                return <ExternalResult key={result.id} result={result} linked={Boolean(local)} downloading={downloading}
                  preview={() => local ? playLocal(local) : void onPreview(result)}
                  action={() => local ? playLocal(local) : void onDownload(result)} />;
              })}
            </div>
          )}
        </section>
      )}
      <section className='search-section search-local'>
        <header className='search-heading'>
          <h2>{localTitle}</h2>
          <p>{noLocal ? 'Совпадений нет' : tab === 'all' ? 'Результаты по категориям' : 'Доступно без сети'}</p>
        </header>
        {localLoading && <p className='kit-message' role='status'>Ищем в медиатеке…</p>}
        {localError && <p className='kit-message' role='alert'>{localError}</p>}
        {noLocal && (
          <div className='search-empty'>
            <h3>Нет локальных результатов</h3>
            <p>Загруженные треки, альбомы и плейлисты появятся здесь.</p>
          </div>
        )}
        {tab === 'all' ? (
          <>
            <div className='track-collection' data-layout='grid'>
              {localTracks.slice(0, 2).map((track) => <Track key={track.id} track={track} onPlay={playLocal} />)}
              {collections.flatMap(({ kind, items, route }) => items.slice(0, 2).map((item) => (
                <CollectionCard key={`${kind}:${item.id}`} item={item} kind={kind}
                  open={() => void navigate({ to: route, search: { collection: item.id } })} />
              )))}
            </div>
            <div className='search-more'>
              {(tracks.hasMore || localTracks.length > 2) &&
                <Button size='small' onClick={() => void navigate({ search: { q, tab: 'tracks', request } })}>Все треки</Button>}
              {collections.filter(({ items }) => items.length > 2).map(({ tab: value }) => (
                <Button key={value} size='small' onClick={() => void navigate({ search: { q, tab: value, request } })}>
                  {{ playlists: 'Все плейлисты', artists: 'Все исполнители', albums: 'Все альбомы' }[value]}
                </Button>
              ))}
            </div>
          </>
        ) : (
          <>
            {localTracks.length > 0 && (
              <div className='track-collection' data-layout='grid'>
                {localTracks.map((track) => <Track key={track.id} track={track} onPlay={playLocal} />)}
              </div>
            )}
            {tracks.hasMore && <Button size='small' className='justify-self-start' onClick={tracks.loadMore}>Показать ещё</Button>}
            {collections.map(({ kind, items, route }) => items.length > 0 && (
              <CollectionList key={kind} items={items} kind={kind} empty='' search
                open={(collection) => void navigate({ to: route, search: { collection } })} />
            ))}
          </>
        )}
      </section>
    </section>
  );
}

function PreviewControls({ preview }: { preview: AudioPreview }) {
  const [drag, setDrag] = useState<number>();
  const { id, title, artist, loading, playing, duration } = preview;
  const position = drag ?? preview.position;
  return (
    <div className='flex flex-wrap items-center gap-3 rounded-xl border border-(--yap-border) bg-(--yap-surface) p-3'
      role='group' aria-label='Предпросмотр'>
      <div className='min-w-0'>
        <p className='truncate font-semibold' title={title}>{title}</p>
        {artist && <p className='track-artist truncate'>{artist}</p>}
      </div>
      {loading ? <YtDlpChecking label='Загружаем предпросмотр…' /> : (
        <div className='flex min-w-0 grow flex-wrap items-center gap-3'>
          <Button size='small' onClick={() => setPreviewPlaying(id, !playing)}>
            {playing ? 'Пауза предпросмотра' : 'Продолжить предпросмотр'}
          </Button>
          <span className='text-xs tabular-nums'>{formatPlaybackTime(position)}</span>
          <SeekBar label='Позиция предпросмотра' className='min-w-24 grow'
            value={position} max={Math.max(duration ?? 0, 0.1)} disabled={!duration}
            onSeek={setDrag} onSeekEnd={(value) => { setDrag(undefined); seekPreview(id, value); }} />
          {duration !== null && <span className='text-xs tabular-nums'>{formatPlaybackTime(duration)}</span>}
        </div>
      )}
      <Button size='small' onClick={() => closePreview(id)}>
        {loading ? 'Отменить предпросмотр' : 'Закрыть предпросмотр'}
      </Button>
    </div>
  );
}

const downloadLabels = {
  preparing: 'Подготовка…',
  downloading: 'Скачиваем аудио…',
  processing: 'Обрабатываем аудио…',
  waitingForLibrary: 'Ждём окончания обновления медиатеки…',
  importing: 'Добавляем трек в медиатеку…',
};

function DownloadStatus() {
  const active = useMusicDownload((state) => state.active);
  if (!active) return null;

  const { progress, cancelling } = active;
  const phase = progress?.phase ?? 'preparing';
  const percent = progress?.phase === 'downloading' ? progress.percent : null;
  const label = cancelling ? 'Отменяем загрузку…' : downloadLabels[phase];

  return (
    <div className='flex flex-wrap items-center gap-3 text-(--yap-muted)'>
      <span role='status'>{label}</span>
      {phase === 'downloading' && !cancelling && (
        <>
          <Progress className='w-32' max={100} value={percent} label='Ход загрузки' />
          {percent !== null && <span>{Math.round(percent)}%</span>}
        </>
      )}
      {phase !== 'importing' && (
        <Button size='small' variant='danger' disabled={!progress || cancelling}
          aria-label='Отменить загрузку'
          onClick={() => void cancelMusicDownload().catch((error) => toast.error('Не удалось отменить загрузку', error))}>
          Отменить
        </Button>
      )}
    </div>
  );
}

const isGatekeeperBlock = (error: unknown) => error instanceof YtDlpError && error.state === 'gatekeeperBlocked';
