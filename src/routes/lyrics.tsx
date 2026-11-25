import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { createFileRoute, Link } from '@tanstack/react-router';
import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { LyricsOffsetControl, previewLyricsOffset, savedLyricsOffset, useLyricsOffsetPreview } from '@/components/LyricsOffsetControl';
import { MediaDetails } from '@/components/MediaDetails';
import { Select, type SelectOption } from '@/components/Select/Select';
import type { Track } from '@/types/Track';
import { useLibraryTrack } from '@/hooks/useLibrary';
import { usePlayerStore, useSettingsStore } from '@/stores';
import { geniusReady, lyricaReady } from '@/stores/settingsStore';
import { getLocalLyrics, localLyricsText, lookupLyrics, lookupPreferredLyrics, lyricaError, lyricaProviders, onlineLyricsSearch, parseLrc, saveProviderLyrics, type LocalLyricsData, type LyricsResult, type LyricaProvider } from '@/utils/lyrica';
import { geniusError, geniusPageUrl } from '@/utils/genius';
import { usePlayerVideo } from '@/utils/playerVideo';
import { describePlayback } from '@/utils/trackMetadata';
import { toast } from '@/utils/toast';

export const Route = createFileRoute('/lyrics')({ component: Lyrics });

type Provider = 'genius' | LyricaProvider;
type Source = 'local' | Provider;
type Line = { time: number; content: string };

const sources: readonly SelectOption<Source>[] = [
  lyricaProviders[0],
  { value: 'local', label: 'Локальный текст' },
  ...lyricaProviders.slice(1),
  { value: 'genius', label: 'Genius' },
];

function LyricsReader({ path, text, lines, offsetMs, onLyrics }: {
  path: string;
  text: string;
  lines: Line[];
  offsetMs: number;
  onLyrics: (visible: boolean) => void;
}) {
  const time = usePlayerStore((state) => state.currentTime);
  const seek = usePlayerStore((state) => state.commitSeek);
  const scrollPanel = useRef<HTMLDivElement>(null);
  const activeLine = useRef<HTMLButtonElement>(null);
  const previousLines = useRef<Line[]>();

  let active = -1;
  for (let index = 0; index < lines.length && lines[index].time + offsetMs / 1000 <= time; index += 1) active = index;

  const hasLyrics = lines.length > 0 || !!text.trim();
  useEffect(() => {
    onLyrics(hasLyrics);
    return () => onLyrics(false);
  }, [hasLyrics, onLyrics]);

  useEffect(() => {
    const panel = scrollPanel.current;
    const line = activeLine.current;
    if (!panel || !lines.length) return;
    const bounds = panel.getBoundingClientRect();
    const item = line?.getBoundingClientRect();
    const position = item ? panel.scrollTop + item.top + item.height / 2 - bounds.top - bounds.height / 2 : 0;
    const top = Math.max(0, Math.min(position, panel.scrollHeight - panel.clientHeight));
    const animate = previousLines.current === lines && !matchMedia('(prefers-reduced-motion: reduce)').matches;
    previousLines.current = lines;
    panel.scrollTo({ top, behavior: animate ? 'smooth' : 'instant' });
    let width = panel.clientWidth;
    let height = panel.clientHeight;
    const resize = new ResizeObserver(() => {
      if (!line || (width === panel.clientWidth && height === panel.clientHeight)) return;
      width = panel.clientWidth;
      height = panel.clientHeight;
      const panelBounds = panel.getBoundingClientRect();
      const lineBounds = line.getBoundingClientRect();
      const centered = panel.scrollTop + lineBounds.top + lineBounds.height / 2 - panelBounds.top - panelBounds.height / 2;
      const top = Math.max(0, Math.min(centered, panel.scrollHeight - panel.clientHeight));
      panel.scrollTo({ top, behavior: 'instant' });
    });
    resize.observe(panel);
    return () => resize.disconnect();
  }, [active, lines, path]);

  return (
    <div ref={scrollPanel} className='lyrics-lines' data-synced={lines.length > 0} aria-label='Текст песни'>
      {lines.length > 0 ? lines.map((line, index) => (
        <button key={index} type='button' ref={active === index ? activeLine : undefined}
          aria-current={active === index ? 'true' : undefined} onClick={() => seek(path, Math.max(0, line.time + offsetMs / 1000))}>
          {line.content || '♪'}
        </button>
      )) : <p>{text}</p>}
    </div>
  );
}

function LocalLyrics({ track, automatic, onLyrics, onTryLrclib, offsetMs }: {
  track: Track;
  automatic: boolean;
  offsetMs: number;
  onLyrics: (visible: boolean) => void;
  onTryLrclib?: () => void;
}) {
  const [local, setLocal] = useState<LocalLyricsData | null>();
  const text = localLyricsText(track, local ?? null, automatic);
  const search = useSettingsStore((settings) => automatic && local ? onlineLyricsSearch(settings, text, local.fetched) : undefined);
  useEffect(() => {
    let active = true;
    const read = () => void getLocalLyrics(track.path)
      .then((result) => { if (active) setLocal(result); })
      .catch(() => { if (active) setLocal(null); });
    const listener = listen<string>('library:lyrics', ({ payload }) => {
      if (payload === track.path) read();
    });
    void listener.then(read, read);
    return () => {
      active = false;
      void listener.then((unlisten) => unlisten(), () => undefined);
    };
  }, [track.path]);
  if (local === undefined) return <div className='lyrics-status'><LoadingIndicator label='Чтение локального текста' /></div>;
  if (search && local) return <ProviderLookup track={track} provider='auto' fallbackText={search === 'synchronized' ? text : undefined}
    expectedFetched={local.fetched} offsetMs={offsetMs} onLyrics={onLyrics} onTryLrclib={onTryLrclib} />;
  if (text) return <StoredLyrics path={track.path} text={text} offsetMs={offsetMs} onLyrics={onLyrics} />;
  return <div className='lyrics-status'>Для этого трека нет локального текста.</div>;
}

function StoredLyrics({ path, text, offsetMs, onLyrics }: { path: string; text: string; offsetMs: number; onLyrics: (visible: boolean) => void }) {
  const lines = useMemo(() => parseLrc(text), [text]);
  return <LyricsReader path={path} text={text} lines={lines} offsetMs={offsetMs} onLyrics={onLyrics} />;
}

function ProviderLookup({ track, provider, expectedFetched, fallbackText, onLyrics, onTryLrclib, offsetMs }: {
  track: Track;
  provider: Provider;
  expectedFetched?: string | null;
  fallbackText?: string;
  offsetMs: number;
  onLyrics: (visible: boolean) => void;
  onTryLrclib?: () => void;
}) {
  const [state, setState] = useState<
    { kind: 'loading' | 'missing' }
    | { kind: 'error'; message: string; needsToken: boolean; page?: string }
    | ({ kind: 'found' } & LyricsResult)
  >({ kind: 'loading' });
  const [attempt, setAttempt] = useState(0);
  const artist = track.artists.join(', ');
  const lyricaAvailable = useSettingsStore((settings) => provider === 'auto' && lyricaReady(settings));
  const geniusAvailable = useSettingsStore((settings) => provider === 'auto' && !fallbackText && geniusReady(settings));

  useEffect(() => {
    let active = true;
    setState({ kind: 'loading' });
    if (!artist.trim()) return;
    const lookup = provider === 'auto'
      ? lookupPreferredLyrics(track, fallbackText ? 'synchronized' : 'preferred')
      : lookupLyrics(track, provider);
    void lookup.then((result) => {
      if (!active) return;
      if (!result) {
        setState({ kind: 'missing' });
        return;
      }

      setState({ kind: 'found', ...result });
      void saveProviderLyrics(track.path, result.content, expectedFetched)
        .catch((error) => console.warn('Could not save track lyrics', error));
    }).catch((error) => {
      if (!active) return;
      const failure = provider === 'auto' ? error as { provider: string; reason: unknown }
        : { provider, reason: error };
      const genius = failure?.provider === 'genius';
      setState({
        kind: 'error',
        message: genius ? geniusError(failure.reason) : lyricaError(failure?.reason),
        needsToken: genius && (failure.reason === 'notConfigured' || failure.reason === 'unauthorized'),
        page: genius ? geniusPageUrl(failure.reason) : undefined,
      });
    });
    return () => { active = false; };
  }, [track.path, track.name, artist, provider, expectedFetched, fallbackText, attempt, lyricaAvailable, geniusAvailable]);

  if (fallbackText && state.kind !== 'found') {
    return <div className='relative grid min-h-0 grid-rows-[minmax(0,1fr)]'>
      <StoredLyrics path={track.path} text={fallbackText} offsetMs={offsetMs} onLyrics={onLyrics} />
      {state.kind === 'loading' && artist.trim() &&
        <LoadingIndicator label='Поиск синхронизированного текста' className='absolute right-2 top-2' />}
    </div>;
  }
  if (!artist.trim()) return <p className='lyrics-status'>Для поиска текста нужен исполнитель.</p>;

  if (state.kind === 'found') {
    const { label, instrumental, content } = state;
    return (
      <div className='lyrics-result'>
        <p className='lyrics-credit' role='status'>Источник — {label}</p>
        {instrumental && !content.lines.length && !content.text.trim()
          ? <div className='lyrics-status'>Инструментальный трек</div>
          : <LyricsReader path={track.path} text={content.text} lines={content.lines} offsetMs={offsetMs} onLyrics={onLyrics} />}
      </div>
    );
  }
  const page = state.kind === 'error' ? state.page : undefined;
  return (
    <div className='lyrics-status'>
      {state.kind === 'loading' && attempt === 0 && <LoadingIndicator label='Поиск текста песни' />}
      {state.kind === 'missing' && <p role='status'>Текст не найден. Можно попробовать другой источник.</p>}
      {state.kind === 'error' && <p className='kit-error' role='alert'>{state.message}</p>}
      {state.kind === 'error' && state.needsToken && <Link to='/settings' search={{ category: 'tools' }}>Настройки Genius</Link>}
      {page ? <div className='lyrics-status-actions'>
        <Button variant='primary' size='small' icon='Open' onClick={() => void openUrl(page)
          .catch(() => toast.error('Не удалось открыть страницу Genius'))}>Открыть в Genius</Button>
        {onTryLrclib && <Button size='small' icon='Search' onClick={onTryLrclib}>Искать в LRCLIB</Button>}
      </div> : (attempt > 0 || state.kind !== 'loading') && <Button size='small' icon='Search'
        pending={state.kind === 'loading'} onClick={() => setAttempt(attempt + 1)}>Повторить поиск</Button>}
    </div>
  );
}

function Lyrics() {
  const track = usePlayerStore((state) => state.currentTrack);
  const nextPath = usePlayerStore((state) => state.nextTrackPath);
  const clipPlaying = usePlayerStore((state) => state.clipId !== null && state.currentTrackPath === state.currentTrack?.path);
  const videoPath = usePlayerVideo((state) => state.request?.path);
  const lyricaEnabled = useSettingsStore(lyricaReady);
  const geniusEnabled = useSettingsStore(geniusReady);
  const preferred = useSettingsStore((state) => state.lyricsSource);
  const [selected, setSource] = useState<Source>('auto');
  const tryLrclib = useCallback(() => setSource('lrclib'), []);
  const options = sources.filter(({ value }) => value === 'auto' || value === 'local'
    || (value === 'genius' ? geniusEnabled : lyricaEnabled));
  const source = options.some(({ value }) => value === selected) ? selected : 'auto';
  const path = track?.path;
  const withVideo = path !== undefined && videoPath === path;
  const timeline = clipPlaying ? 'clip' : 'track';
  const preview = useLyricsOffsetPreview((state) => state[timeline]?.path === path ? state[timeline]?.offsetMs : undefined);
  const storedOffset = savedLyricsOffset(track, timeline);
  const previewOffset = useCallback((offsetMs?: number) => {
    if (path) previewLyricsOffset(path, offsetMs);
  }, [path]);
  useEffect(() => {
    if (path && preview === storedOffset) previewLyricsOffset(path, undefined, timeline);
  }, [path, preview, storedOffset, timeline]);
  const offsetMs = preview ?? storedOffset;
  const sourceKey = JSON.stringify([path, source, source === 'auto' ? preferred : null]);
  const [visibleLyrics, setVisibleLyrics] = useState<string>();
  const onLyrics = useCallback((visible: boolean) => {
    setVisibleLyrics((previous) => visible ? sourceKey : previous === sourceKey ? undefined : previous);
  }, [sourceKey]);

  const nextTrack = useLibraryTrack(nextPath === path || visibleLyrics !== sourceKey ? null : nextPath);
  if (!track) return <p className='kit-message'>Выберите трек, чтобы увидеть текст.</p>;
  const next = visibleLyrics === sourceKey && (nextPath === path ? track : nextTrack);

  return (
    <section className='collection-details lyrics-details'>
      <MediaDetails item={{ name: track.name, subtitle: track.artists.join(', '), icon: 'Track', path, track }}
        metadata={describePlayback([track])}
        next={next ? { name: next.name, subtitle: next.artists.join(', '), icon: 'Track', path: next.path, track: next } : undefined} />
      <div className='lyrics-panel' data-with-video={withVideo}>
        {!withVideo && <div className='lyrics-toolbar'>
          <label htmlFor='lyrics-source'>Источник текста</label>
          <Select id='lyrics-source' label='Источник текста' value={source} options={options} onChange={setSource} />
          <LyricsOffsetControl key={path} track={track} onPreview={previewOffset} />
        </div>}
        {source === 'auto' || source === 'local'
          ? <LocalLyrics key={sourceKey} track={track} automatic={source === 'auto'} offsetMs={offsetMs} onLyrics={onLyrics}
            onTryLrclib={lyricaEnabled ? tryLrclib : undefined} />
          : <ProviderLookup key={sourceKey} track={track} provider={source} offsetMs={offsetMs} onLyrics={onLyrics}
            onTryLrclib={lyricaEnabled ? tryLrclib : undefined} />}
      </div>
    </section>
  );
}
