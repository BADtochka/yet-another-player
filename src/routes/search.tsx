import { invoke } from '@tauri-apps/api/core';
import { MediaCard } from '@/components/MediaCard';
import type { AlbumRelease } from '@/hooks/useRemoteCatalog';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { ExternalResult } from '@/components/ExternalResult';
import { CollectionCard, CollectionList } from '@/components/Collections';
import { Track } from '@/components/Track';
import { TrackArtistLinks } from '@/components/TrackLinks';
import { TrackPreview } from '@/components/TrackPreview';
import { YtDlpChecking } from '@/components/YtDlpFeedback';
import { useSettingsStore } from '@/stores';
import { useCollections, useTrackPages, useExternalTracks, type TrackQuery } from '@/hooks/useLibrary';
import { playSearchTrack } from '@/stores/playerStore';
import { addSearchHistory } from '@/stores/searchHistory';
import type { Track as LibraryTrack } from '@/types/Track';
import { downloadMusic, useMusicDownload } from '@/utils/musicDownload';
import { closePlayerVideo, showPlayerVideo } from '@/utils/playerVideo';
import { toast } from '@/utils/toast';
import { runYtDlpSearch, useYtDlpStatus, YtDlpError, type MusicSearchResult } from '@/utils/ytDlp';

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
  const [preview, setPreview] = useState<MusicSearchResult>();
  const checkingTool = useYtDlpStatus((state) => state.checking);
  const activeVideo = useMusicDownload((state) => state.active?.videoId);
  const query = q.trim();
  const remoteVisible = query.length >= 2;
  const showRemoteTracks = remoteVisible && (tab === 'all' || tab === 'tracks');
  const showRemoteCatalog = remoteVisible && !/^https?:\/\//iu.test(query);
  const remoteKey = JSON.stringify([query, request, searchAttempt]);
  const currentRemote = remote?.key === remoteKey ? remote : undefined;
  const external = currentRemote?.items ?? [];
  const externalIds = external.map(({ id }) => id);
  const links = useExternalTracks(externalIds);
  const remoteError = currentRemote?.error;
  const searching = remoteVisible && !currentRemote;
  const visible = (kind: Tab) => query !== '' && (tab === 'all' || tab === kind);
  const trackQuery: TrackQuery = { text: query, sort: 'path' };
  const tracks = useTrackPages(trackQuery, visible('tracks'));
  const albums = useCollections('album', query, visible('albums'));
  const artists = useCollections('artist', query, visible('artists'));
  const playlists = useCollections('playlist', query, visible('playlists'));

  useEffect(() => {
    if (!showRemoteTracks || currentRemote) return;
    let active = true;
    void runYtDlpSearch(query).then((items) => {
      if (active) setRemote({ key: remoteKey, items });
    }).catch((error) => {
      if (active) {
        setRemote({ key: remoteKey, items: [], error: error instanceof YtDlpError ? error.message : 'Внешний поиск недоступен.' });
      }
    });
    return () => { active = false; };
  }, [remoteKey, showRemoteTracks, currentRemote]);

  const indexedIds = new Set(tracks.tracks.map((track) => track.id));
  const linkedTracks = new Map(Object.values(links?.data ?? {}).map((track) => [track.id, track]));
  const localTracks = visible('tracks')
    ? [...tracks.tracks, ...Array.from(linkedTracks.values()).filter((track) => !indexedIds.has(track.id))]
    : [];
  const localAlbums = albums?.data?.filter((item) => !item.shortRelease) ?? [];
  const localFound = !!(localTracks.length || localAlbums.length || artists?.data?.length || playlists?.data?.length);
  const localLoading = tracks.loading || (visible('tracks') && external.length > 0 && !links)
    || (visible('albums') && !albums) || (visible('artists') && !artists)
    || (visible('playlists') && !playlists);
  const localError = tracks.error ?? albums?.error ?? artists?.error ?? playlists?.error;

  const playResult = async (track: LibraryTrack, videoId?: string) => {
    try {
      if (!await playSearchTrack(trackQuery, track, externalIds, videoId !== undefined)) return;
      addSearchHistory({ query, trackId: track.id, title: track.name, artist: track.artists.join(', ') });
      if (videoId) await showPlayerVideo({ ...track, clipVideoId: videoId });
    } catch (error) {
      toast.error('Не удалось открыть очередь поиска', error);
    }
  };

  const onDownload = async (result: MusicSearchResult, video = false) => {
    if (useMusicDownload.getState().active) return;
    if (useSettingsStore.getState().musicFolders.length === 0) {
      toast.error('Добавьте папку медиатеки в настройках');
      return;
    }
    try {
      const track = await downloadMusic(result.id, 0, video);
      await playResult(track, video ? result.id : undefined);
    } catch (error) {
      if (error instanceof YtDlpError && error.state === 'downloadCancelled') {
        toast.info(error.message);
      } else if (!isGatekeeperBlock(error)) {
        toast.error('Не удалось добавить трек', error);
      }
    }
  };

  const collections = [
    { kind: 'playlist', items: playlists?.data ?? [], route: '/playlists', tab: 'playlists' },
    { kind: 'artist', items: artists?.data ?? [], route: '/artists', tab: 'artists' },
    { kind: 'album', items: localAlbums, route: '/albums', tab: 'albums' },
  ] as const;
  const localTitle = {
    all: 'Локальная медиатека', tracks: 'Локальные треки', playlists: 'Локальные плейлисты',
    artists: 'Локальные исполнители', albums: 'Локальные альбомы',
  }[tab];
  const noRemote = remoteVisible && !searching && !remoteError && external.length === 0;
  const focusSearch = () => {
    const input = document.querySelector<HTMLInputElement>('.app-search input');
    input?.focus();
    input?.select();
  };

  if (!query) return null;

  return (
    <section className='search-page'>
      <header className='search-heading'>
        <h1>Результаты для «{query}»</h1>
      </header>
      <div className='search-tabs' role='group' aria-label='Категория результатов'>
        {tabs.map(([value, title]) => (
          <Button key={value} variant={tab === value ? 'primary' : 'secondary'} size='small' aria-pressed={tab === value}
            onClick={() => void navigate({ search: { q, tab: value, request } })}>{title}</Button>
        ))}
      </div>
      {(localFound || localLoading || localError) && <section className='search-section search-local'>
        {localFound && <header className='search-heading'><h2>{localTitle}</h2></header>}
        {localLoading && <LoadingIndicator className='kit-message' label='Поиск в медиатеке' />}
        {localError && <p className='kit-message' role='alert'>{localError}</p>}
        {tab === 'all' ? (
          <>
            <div className='track-collection' data-layout='grid'>
              {localTracks.slice(0, 2).map((track) => <Track key={track.id} track={track} onPlay={(track) => void playResult(track)} />)}
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
                {localTracks.map((track) => <Track key={track.id} track={track} onPlay={(track) => void playResult(track)} />)}
              </div>
            )}
            {tracks.hasMore && <Button size='small' className='justify-self-start' pending={tracks.loadingMore} onClick={tracks.loadMore}>Показать ещё</Button>}
            {collections.map(({ kind, items, route }) => items.length > 0 && (
              <CollectionList key={kind} items={items} kind={kind} empty='' search
                open={(collection) => void navigate({ to: route, search: { collection } })} />
            ))}
          </>
        )}
      </section>}
      {showRemoteTracks && (
        <section className='search-section search-remote'>
          <header className='search-heading'>
            <h2>Треки из внешних источников</h2>
          </header>
          {noRemote && (
            <div className='search-empty'>
              <h3>Ничего не найдено</h3>
              <p>Попробуйте более короткий запрос, имя автора или прямую ссылку.</p>
              <Button size='small' variant='primary' onClick={focusSearch}>Изменить запрос</Button>
            </div>
          )}
          {searching && <YtDlpChecking label={checkingTool ? undefined : 'Ищем во внешних источниках…'} />}
          {remoteError && (
            <div className='search-empty' role='status'>
              <h3>Внешний поиск недоступен</h3>
              <p>{remoteError}</p>
              <Button size='small' onClick={() => setSearchAttempt((attempt) => attempt + 1)}>Повторить поиск</Button>
            </div>
          )}
          {external.length > 0 && (
            <div className='search-external'>
              {external.map((result) => {
                const local = links?.data?.[result.id];
                return <ExternalResult key={result.id} result={result} localPath={local?.path}
                  disabled={!local && activeVideo !== undefined}
                  action={() => void (local ? playResult(local) : onDownload(result))}
                  preview={() => { closePlayerVideo(); setPreview(result); }}
                  video={() => void (local ? playResult(local, result.id) : onDownload(result, true))} />;
              })}
            </div>
          )}
        </section>
      )}
      {showRemoteCatalog && <>
        <ExternalCatalogSection key={remoteKey + ':artists'} category='artists' query={query} visible={visible('artists')} />
        <ExternalCatalogSection key={remoteKey + ':albums'} category='albums' query={query} visible={visible('albums')} />
      </>}
      {preview && <TrackPreview key={preview.id} result={preview} onDismiss={() => setPreview(undefined)} />}
    </section>
  );
}

const isGatekeeperBlock = (error: unknown) => error instanceof YtDlpError && error.state === 'gatekeeperBlocked';

type ExternalCatalogItem =
  | { type: 'artist'; id: number; name: string; genre: string | null }
  | ({ type: 'album' } & AlbumRelease);

function ExternalCatalogSection({ category, query, visible }: {
  category: 'artists' | 'albums';
  query: string;
  visible: boolean;
}) {
  const navigate = Route.useNavigate();
  const [result, setResult] = useState<{ items: ExternalCatalogItem[]; error?: boolean }>();
  const [attempt, setAttempt] = useState(0);
  const heading = category === 'artists' ? 'Исполнители из внешних источников' : 'Альбомы из внешних источников';

  useEffect(() => {
    if (!visible || result) return;
    let active = true;
    void invoke<ExternalCatalogItem[]>('library_search_catalog', { query, category })
      .then((items) => { if (active) setResult({ items }); })
      .catch(() => { if (active) setResult({ items: [], error: true }); });
    return () => { active = false; };
  }, [query, category, attempt, visible, result]);

  if (!visible) return null;

  return (
    <section className='search-section'>
      <header className='search-heading'><h2>{heading}</h2></header>
      {!result && <LoadingIndicator className='kit-message' label='Поиск во внешнем каталоге' />}
      {result?.error && (
        <div className='search-empty' role='status'>
          <p>Внешний каталог недоступен.</p>
          <Button size='small' onClick={() => { setResult(undefined); setAttempt(attempt + 1); }}>Повторить поиск</Button>
        </div>
      )}
      {result && !result.error && !result.items.length && <p className='kit-message'>Ничего не найдено.</p>}
      {!!result?.items.length && (
        <div className='search-external'>
          {result.items.map((item) => item.type === 'artist'
            ? <MediaCard key={item.id} kind='artist' icon='User' title={item.name} subtitle={item.genre}
                onClick={() => void navigate({ to: '/artists', search: { artist: item.name } })} />
            : <MediaCard key={item.reference.id} kind='album' icon='Album' title={item.title} subtitle={item.artist}
                subtitleLinks={item.artist ? <TrackArtistLinks artists={[item.artist]} allowExternal /> : undefined}
                coverSrc={item.artwork}
                onClick={() => void navigate({ to: '/albums', search: { release: { ...item, artistId: item.artist.trim().toLowerCase() } } })} />)}
        </div>
      )}
    </section>
  );
}
