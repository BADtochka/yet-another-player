import { listen } from '@tauri-apps/api/event';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { createFileRoute, Link } from '@tanstack/react-router';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { MediaDetails } from '@/components/MediaDetails';
import { Select, type SelectOption } from '@/components/Select/Select';
import type { Track } from '@/types/Track';
import { useLibraryTrack } from '@/hooks/useLibrary';
import { usePlayerStore, useSettingsStore } from '@/stores';
import { geniusError, geniusIsConfigured, geniusSearch, type GeniusSong } from '@/utils/genius';
import { getLocalLyrics, isSingleTimedLine, lookupLyricaLyrics, lyricaError, lyricaProviders, parseLrc, readProviderLyrics, saveProviderLyrics, type LocalLyricsData, type LyricaLyrics, type LyricaProvider } from '@/utils/lyrica';

export const Route = createFileRoute('/lyrics')({ component: Lyrics });

type Source = 'local' | 'geniusPage' | LyricaProvider;
type Line = { time: number; content: string };

const sources: readonly SelectOption<Source>[] = [
  { value: 'local', label: 'Локальный текст' },
  ...lyricaProviders,
  { value: 'geniusPage', label: 'Genius · открыть страницу' },
];

function GeniusLookup({ track }: { track: Track }) {
  const [configured, setConfigured] = useState<boolean>();
  const artist = track.artists.join(' ');
  const [loading, setLoading] = useState(true);
  const [results, setResults] = useState<GeniusSong[]>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(undefined);
    setResults(undefined);
    void (async () => {
      try {
        const ready = await geniusIsConfigured();
        if (!active) return;
        setConfigured(ready);
        if (ready) {
          const songs = await geniusSearch(track.name, artist);
          if (active) setResults(songs);
        }
      } catch (reason) {
        if (active) {
          if (reason === 'notConfigured') setConfigured(false);
          else setError(geniusError(reason));
        }
      } finally {
        if (active) setLoading(false);
      }
    })();
    return () => { active = false; };
  }, [track.name, artist]);

  return (
    <div className='genius-lookup'>
      <p>Найдите страницу с текстом на Genius.</p>
      {loading && <LoadingIndicator label='Поиск в Genius' />}
      {configured === false && <Link to='/settings'>Подключить Genius в настройках</Link>}
      {error && <p className='kit-error' role='alert'>{error}</p>}
      {results && (results.length ? (
        <ul aria-label='Результаты поиска Genius'>
          {results.map((song) => <li key={song.url}>
            <Button icon='Open' size='small' onClick={() => void openUrl(song.url).catch(() => setError('Не удалось открыть страницу Genius.'))}>
              {song.title} · {song.artist}
            </Button>
          </li>)}
        </ul>
      ) : <p role='status'>Genius не нашёл совпадений.</p>)}
    </div>
  );
}

function LyricsReader({ path, text, lines }: { path: string; text: string; lines: Line[] }) {
  const time = usePlayerStore((state) => state.currentTime);
  const seek = usePlayerStore((state) => state.commitSeek);
  const scrollPanel = useRef<HTMLDivElement>(null);
  const activeLine = useRef<HTMLButtonElement>(null);
  const previousLines = useRef<Line[]>();

  let active = -1;
  for (let index = 0; index < lines.length && lines[index].time <= time; index += 1) active = index;

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
          aria-current={active === index ? 'true' : undefined} onClick={() => seek(path, line.time)}>
          {line.content || '♪'}
        </button>
      )) : <p>{text}</p>}
    </div>
  );
}

function LocalLyrics({ track }: { track: Track }) {
  const [local, setLocal] = useState<LocalLyricsData | null>();
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
  const embedded = track.lyrics.join('\n');
  const fetched = local?.fetched ?? '';
  // Older builds could store a single timed line in both the catalog and the file tag.
  const stored = [embedded, fetched].filter((text) => text.trim());
  const text = local?.sidecar || stored.find((value) => !isSingleTimedLine(value))
    || stored[0];
  if (text) return <StoredLyrics path={track.path} text={text} />;
  return <div className='lyrics-status'>Для этого трека нет локального текста.</div>;
}

function StoredLyrics({ path, text }: { path: string; text: string }) {
  const lines = useMemo(() => parseLrc(text), [text]);
  return <LyricsReader path={path} text={text} lines={lines} />;
}

function LyricaLookup({ track, provider }: { track: Track; provider: LyricaProvider }) {
  const [state, setState] = useState<
    { kind: 'loading' | 'missing' }
    | { kind: 'error'; message: string }
    | { kind: 'found'; lyrics: LyricaLyrics; content: ReturnType<typeof readProviderLyrics> }
  >({ kind: 'loading' });
  const [attempt, setAttempt] = useState(0);
  const artist = track.artists.join(', ');

  useEffect(() => {
    let active = true;
    setState({ kind: 'loading' });
    if (!artist.trim()) return;
    void lookupLyricaLyrics(track.name, artist, provider).then((result) => {
      if (!active) return;
      if (!result) {
        setState({ kind: 'missing' });
        return;
      }

      const resolved = readProviderLyrics(result);
      setState({ kind: 'found', lyrics: result, content: resolved });
      void saveProviderLyrics(track.path, resolved)
        .catch((error) => console.warn('Could not save track lyrics', error));
    }).catch((reason) => {
      if (active) setState({ kind: 'error', message: lyricaError(reason) });
    });
    return () => { active = false; };
  }, [track.path, track.name, artist, provider, attempt]);

  if (!artist.trim()) return <p className='lyrics-status'>Для поиска текста нужен исполнитель.</p>;

  if (state.kind === 'found') {
    const { lyrics, content } = state;
    const name = lyricaProviders.find((option) => option.value === lyrics.provider)?.label;
    return (
      <div className='lyrics-result'>
        <p className='lyrics-credit' role='status'>{name ? `Источник: ${name}` : 'Источник: Lyrica'}</p>
        {lyrics.instrumental && !content.lines.length && !content.text.trim()
          ? <div className='lyrics-status'>Инструментальный трек</div>
          : <LyricsReader path={track.path} text={content.text} lines={content.lines} />}
      </div>
    );
  }
  return (
    <div className='lyrics-status'>
      {state.kind === 'loading' && attempt === 0 && <LoadingIndicator label='Поиск текста песни' />}
      {state.kind === 'missing' && <p role='status'>Текст не найден. Можно попробовать другой источник.</p>}
      {state.kind === 'error' && <p className='kit-error' role='alert'>{state.message}</p>}
      {(attempt > 0 || state.kind !== 'loading') && <Button size='small' icon='Search' pending={state.kind === 'loading'}
        onClick={() => setAttempt(attempt + 1)}>Повторить поиск</Button>}
    </div>
  );
}

function Lyrics() {
  const track = usePlayerStore((state) => state.currentTrack);
  const nextPath = usePlayerStore((state) => state.nextTrackPath);
  const lyricaEnabled = useSettingsStore((state) => state.lyricaEnabled);
  const [selected, setSource] = useState<Source>('local');
  const options = lyricaEnabled ? sources : sources.filter(({ value }) => value === 'local' || value === 'geniusPage');
  const source = options.some(({ value }) => value === selected) ? selected : 'local';
  const path = track?.path;

  const nextTrack = useLibraryTrack(nextPath === path ? null : nextPath);
  if (!track) return <p className='kit-message'>Выберите трек, чтобы увидеть текст.</p>;
  const next = nextPath === path ? track : nextTrack;

  return (
    <section className='collection-details lyrics-details'>
      <MediaDetails item={{ name: track.name, subtitle: track.artists.join(', '), icon: 'Track', path, track }}
        metadata={[
          ...(track.year ? [{ icon: 'Calendar' as const, label: String(track.year) }] : []),
          ...(track.genre ? [{ icon: 'Melody' as const, label: track.genre }] : []),
        ]}
        next={next && { name: next.name, subtitle: next.artists.join(', '), icon: 'Track', path: next.path, track: next }} />
      <div className='lyrics-panel'>
        <div className='lyrics-toolbar'>
          <label htmlFor='lyrics-source'>Источник текста</label>
          <Select id='lyrics-source' label='Источник текста' value={source} options={options} onChange={setSource} />
        </div>
        {source === 'local' ? <LocalLyrics key={track.path} track={track} />
          : source === 'geniusPage' ? <GeniusLookup key={track.path} track={track} />
            : <LyricaLookup key={`${source}:${track.path}`} track={track} provider={source} />}
      </div>
    </section>
  );
}
