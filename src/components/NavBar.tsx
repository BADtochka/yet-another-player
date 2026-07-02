import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { Album } from '@/components/Icons/Album';
import { Playlist } from '@/components/Icons/Playlist';
import { Settings } from '@/components/Icons/Settings';
import { Track as TrackIcon } from '@/components/Icons/Track';
import { User } from '@/components/Icons/User';
import { Input } from '@/components/Input';
import { Tab } from '@/components/Tab';
import { useMusicStore, usePlayerStore, useSettingsStore } from '@/stores';
import { getTrackFromPath } from '@/utils/getTracks';
import {
  downloadMusic,
  getMusicPreviewUrl,
  type MusicDownloadProgress,
  type MusicSearchResult,
  searchMusic,
} from '@/utils/musicSearch';
import { parseMusic } from '@/utils/parseMusic';

const formatDuration = (seconds: number | null) => {
  if (!seconds) {
    return null;
  }

  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = Math.floor(seconds % 60)
    .toString()
    .padStart(2, '0');

  return `${minutes}:${remainingSeconds}`;
};

export const NavBar = () => {
  const musicFolders = useSettingsStore((state) => state.musicFolders);
  const tracks = useMusicStore((state) => state.tracks);
  const addTrack = useMusicStore((state) => state.addTrack);
  const setCurrentTrack = usePlayerStore((state) => state.setCurrentTrack);
  const setCurrentTime = usePlayerStore((state) => state.setCurrentTime);
  const setPlaying = usePlayerStore((state) => state.setPlaying);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<MusicSearchResult[]>([]);
  const [isFocused, setIsFocused] = useState(false);
  const [isSearching, setIsSearching] = useState(false);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [previewId, setPreviewId] = useState<string | null>(null);
  const [downloadingId, setDownloadingId] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<MusicDownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const searchRef = useRef<HTMLDivElement>(null);
  const searchRequestId = useRef(0);

  const localResults = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();

    if (!normalizedQuery) {
      return [];
    }

    return tracks
      .filter((track) => {
        const haystack = [track.name, track.album, ...track.artists].filter(Boolean).join(' ').toLowerCase();

        return haystack.includes(normalizedQuery);
      })
      .slice(0, 5);
  }, [query, tracks]);

  const showResults =
    isFocused && (query.trim().length > 0 || localResults.length > 0 || results.length > 0 || Boolean(error));

  useEffect(() => {
    const normalizedQuery = query.trim();
    const requestId = searchRequestId.current + 1;
    searchRequestId.current = requestId;
    setPreviewUrl(null);
    setPreviewId(null);
    setError(null);

    if (normalizedQuery.length < 2) {
      setResults([]);
      setIsSearching(false);
      return;
    }

    setIsSearching(true);

    const timeoutId = window.setTimeout(() => {
      searchMusic(normalizedQuery)
        .then((nextResults) => {
          if (searchRequestId.current === requestId) {
            setResults(nextResults);
          }
        })
        .catch((searchError) => {
          if (searchRequestId.current === requestId) {
            setResults([]);
            setError(searchError instanceof Error ? searchError.message : String(searchError));
          }
        })
        .finally(() => {
          if (searchRequestId.current === requestId) {
            setIsSearching(false);
          }
        });
    }, 450);

    return () => window.clearTimeout(timeoutId);
  }, [query]);

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;

      if (!(target instanceof Node) || searchRef.current?.contains(target)) {
        return;
      }

      setIsFocused(false);
    };

    window.addEventListener('pointerdown', onPointerDown);

    return () => window.removeEventListener('pointerdown', onPointerDown);
  }, []);

  const onPreview = async (result: MusicSearchResult) => {
    setError(null);
    setPreviewUrl(null);
    setPreviewId(result.id);

    try {
      const nextPreviewUrl = await getMusicPreviewUrl(result);

      if (!nextPreviewUrl) {
        throw new Error('Не удалось получить предпросмотр');
      }

      setPreviewUrl(nextPreviewUrl);
    } catch (previewError) {
      setError(previewError instanceof Error ? previewError.message : String(previewError));
      setPreviewId(null);
    }
  };

  const onDownload = async (result: MusicSearchResult) => {
    const targetFolder = musicFolders[0];

    if (!targetFolder) {
      setError('Папка медиатеки не выбрана');
      return;
    }

    setError(null);
    setDownloadProgress(null);
    setDownloadingId(result.id);

    try {
      const downloadedPath = await downloadMusic(result, targetFolder, setDownloadProgress);

      if (downloadedPath) {
        addTrack(await getTrackFromPath(downloadedPath));
      } else {
        await parseMusic([targetFolder]);
      }
    } catch (downloadError) {
      setError(downloadError instanceof Error ? downloadError.message : String(downloadError));
    } finally {
      setDownloadingId(null);
    }
  };

  const onLocalTrackClick = (track: (typeof tracks)[number]) => {
    setCurrentTrack(track);
    setCurrentTime(0);
    setPlaying(true);
    setIsFocused(false);
  };

  return (
    <nav className='relative flex flex-wrap items-center gap-3 px-4 font-onest text-white'>
      <div className='flex flex-wrap gap-3'>
        <Tab to='/' icon={<Album size={20} />}>
          Альбомы
        </Tab>
        <Tab to='/tracks' icon={<TrackIcon size={20} />}>
          Треки
        </Tab>
        <Tab to='/artists' icon={<User size={20} />}>
          Исполнители
        </Tab>
        <Tab to='/playlists' icon={<Playlist size={20} />}>
          Плейлисты
        </Tab>
      </div>

      <div
        ref={searchRef}
        className='pointer-events-none absolute left-1/2 top-1/2 z-20 w-[420px] -translate-x-1/2 -translate-y-1/2'
      >
        <Input
          className='pointer-events-auto'
          placeholder='Что-то ищете?'
          aria-label='Поиск'
          value={query}
          onFocus={() => setIsFocused(true)}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />

        {showResults && (
          <div className='pointer-events-auto absolute left-0 right-0 top-[calc(100%+8px)] flex max-h-[520px] flex-col gap-2 overflow-y-auto rounded-lg border border-[#405ea4] bg-[#020a1d] p-2.5 shadow-[0_16px_40px_rgba(0,0,0,0.35)]'>
            {isSearching && <p className='px-2 py-1 text-sm font-medium text-[#8d8d8d]'>Поиск...</p>}

            {localResults.length > 0 && (
              <div className='flex flex-col gap-1'>
                <p className='px-2 text-xs font-medium uppercase text-[#516ba9]'>В медиатеке</p>
                {localResults.map((track) => (
                  <button
                    key={track.path}
                    type='button'
                    className='flex items-center gap-3 rounded-[5px] p-2 text-left transition-colors hover:bg-[#061232]'
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => onLocalTrackClick(track)}
                  >
                    <span className='flex size-10 shrink-0 items-center justify-center rounded-[5px] bg-[#061232] text-[#516ba9]'>
                      <TrackIcon size={18} />
                    </span>
                    <span className='min-w-0 grow'>
                      <span className='block truncate text-sm font-medium text-white'>{track.name}</span>
                      <span className='block truncate text-xs font-medium text-[#8d8d8d]'>
                        {track.artists.length > 0 ? track.artists.join(', ') : 'Unknown artist'}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            )}

            {!isSearching && query.trim().length > 1 && localResults.length === 0 && results.length === 0 && !error && (
              <p className='px-2 py-1 text-sm font-medium text-[#8d8d8d]'>Ничего не найдено</p>
            )}

            {results.length > 0 && (
              <div className='flex flex-col gap-1'>
                <p className='px-2 text-xs font-medium uppercase text-[#516ba9]'>Онлайн</p>
                {results.map((result) => {
                  const duration = formatDuration(result.duration);
                  const isDownloadingThis = downloadingId === result.id;
                  const downloadPercent = isDownloadingThis ? downloadProgress?.percent : null;

                  return (
                    <div key={result.id} className='flex items-center gap-3 rounded-[5px] p-2 transition-colors hover:bg-[#061232]'>
                      {result.thumbnail ? (
                        <img
                          src={result.thumbnail}
                          alt=''
                          className='size-12 shrink-0 rounded-[5px] object-cover'
                          loading='lazy'
                        />
                      ) : (
                        <div className='size-12 shrink-0 rounded-[5px] bg-[#061232]' />
                      )}

                      <div className='min-w-0 grow'>
                        <p className='truncate text-sm font-medium text-white'>{result.title}</p>
                        <p className='truncate text-xs font-medium text-[#8d8d8d]'>
                          {[result.artist, duration].filter(Boolean).join(' · ')}
                        </p>
                      </div>

                      <button
                        type='button'
                        className='shrink-0 text-sm font-medium text-[#506ba9] disabled:opacity-50'
                        disabled={previewId === result.id && !previewUrl}
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => void onPreview(result)}
                      >
                        {previewId === result.id && !previewUrl ? '...' : 'Слушать'}
                      </button>

                      <button
                        type='button'
                        className='shrink-0 text-sm font-medium text-[#506ba9] disabled:opacity-50'
                        disabled={downloadingId !== null}
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => void onDownload(result)}
                      >
                        {downloadPercent !== null && downloadPercent !== undefined
                          ? `${Math.round(downloadPercent)}%`
                          : isDownloadingThis
                            ? '...'
                            : 'Скачать'}
                      </button>
                    </div>
                  );
                })}
              </div>
            )}

            {downloadProgress && (
              <div className='flex flex-col gap-1 px-2 py-1'>
                <div className='h-1.5 overflow-hidden rounded-full bg-[#061232]'>
                  <div
                    className='h-full rounded-full bg-[#405ea4] transition-[width] duration-150'
                    style={{ width: `${downloadProgress.percent ?? 0}%` }}
                  />
                </div>
                <p className='truncate text-xs font-medium text-[#8d8d8d]'>{downloadProgress.status}</p>
              </div>
            )}

            {previewUrl && <audio className='w-full' src={previewUrl} controls autoPlay />}

            {error && <p className='select-text px-2 py-1 text-sm font-medium text-[#da3e3e]'>{error}</p>}
          </div>
        )}
      </div>

      <Tab to='/settings' icon={<Settings size={20} />} className='ml-auto'>
        Настройки
      </Tab>
    </nav>
  );
};
