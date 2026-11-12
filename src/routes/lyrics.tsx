import { createFileRoute, Link } from '@tanstack/react-router';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { MediaDetails } from '@/components/MediaDetails';
import { Select, type SelectOption } from '@/components/Select/Select';
import type { Track } from '@/types/Track';
import { tracksByPaths } from '@/hooks/useLibrary';
import { usePlayerStore } from '@/stores';
import { geniusError, geniusIsConfigured, geniusSearch, type GeniusSong } from '@/utils/genius';
import { lookupLyricaLyrics, lyricaError, lyricaProviders, type LyricaLyrics, type LyricaProvider } from '@/utils/lyrica';

export const Route = createFileRoute('/lyrics')({ component: Lyrics });

type Source = 'local' | 'geniusPage' | LyricaProvider;
type Line = { time: number; content: string };

const sources: readonly SelectOption<Source>[] = [
  { value: 'local', label: 'Локальный текст' },
  ...lyricaProviders,
  { value: 'geniusPage', label: 'Genius · открыть страницу' },
];

function parseLrc(text: string) {
  const offset = Number(text.match(/^\[offset:\s*([+-]?\d+)\]/im)?.[1] ?? 0) / 1000;
  return text.split(/\r?\n/).flatMap((line) => {
    const prefix = line.match(/^(?:\[\d+:\d{2}(?:[.:]\d+)?\])+/)?.[0];
    if (!prefix) return [];
    const content = line.slice(prefix.length).trim();
    return Array.from(prefix.matchAll(/\[(\d+):(\d{2})(?:[.:](\d+))?\]/g), ([, minutes, seconds, fraction]) => ({
      time: Math.max(0, Number(minutes) * 60 + Number(seconds) + Number(`0.${fraction ?? '0'}`) + offset),
      content,
    }));
  }).sort((a, b) => a.time - b.time);
}

function GeniusLookup({ track }: { track: Track }) {
  const [configured, setConfigured] = useState<boolean>();
  const [loading, setLoading] = useState(false);
  const [results, setResults] = useState<GeniusSong[]>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    let active = true;
    void geniusIsConfigured().then((ready) => {
      if (active) setConfigured(ready);
    }).catch((reason) => {
      if (active) setError(geniusError(reason));
    });
    return () => { active = false; };
  }, []);

  async function search() {
    setLoading(true);
    setError(undefined);
    setResults(undefined);
    try {
      setResults(await geniusSearch(track.name, track.artists.join(' ')));
    } catch (reason) {
      if (reason === 'notConfigured') setConfigured(false);
      else setError(geniusError(reason));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className='genius-lookup'>
      <p>Найдите страницу с текстом на Genius.</p>
      {configured === true && (
        <Button icon='Search' disabled={loading} onClick={() => void search()}>
          {loading ? 'Ищем в Genius…' : 'Найти в Genius'}
        </Button>
      )}
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
  let active = -1;
  for (let index = 0; index < lines.length && lines[index].time <= time; index += 1) active = index;

  useEffect(() => {
    const panel = scrollPanel.current;
    const line = activeLine.current;
    if (panel && line) {
      const bounds = panel.getBoundingClientRect();
      const item = line.getBoundingClientRect();
      panel.scrollTop += item.top + item.height / 2 - bounds.top - bounds.height / 2;
    } else if (panel) {
      panel.scrollTop = 0;
    }
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

function LocalLyrics({ track, onSearch }: { track: Track; onSearch: () => void }) {
  const [local, setLocal] = useState<string | null>();
  useEffect(() => {
    let active = true;
    void invoke<string | null>('get_lyrics', { path: track.path })
      .then((text) => { if (active) setLocal(text); })
      .catch(() => { if (active) setLocal(null); });
    return () => { active = false; };
  }, [track.path]);
  const text = local?.trim() ? local : track.lyrics.join('\n');
  const lines = useMemo(() => parseLrc(text), [text]);
  if (text.trim()) return <LyricsReader path={track.path} text={text} lines={lines} />;
  if (local === undefined) return <div className='lyrics-status' role='status'>Читаем локальный текст…</div>;
  return (
    <div className='lyrics-status'>
      <p>Для этого трека нет локального текста.</p>
      <Button size='small' icon='Search' onClick={onSearch}>Выбрать Lyrica</Button>
    </div>
  );
}

function LyricaLookup({ track, provider }: { track: Track; provider: LyricaProvider }) {
  const [state, setState] = useState<
    { kind: 'idle' | 'loading' | 'missing' }
    | { kind: 'error'; message: string }
    | { kind: 'found'; lyrics: LyricaLyrics }
  >({ kind: 'idle' });
  const lyrics = state.kind === 'found' ? state.lyrics : undefined;
  const lines = useMemo(() => lyrics?.lines.map((line) => ({ time: line.startMs / 1000, content: line.text })) ?? [], [lyrics]);
  const artist = track.artists.join(', ');

  async function search() {
    if (state.kind === 'loading') return;
    setState({ kind: 'loading' });
    try {
      const lyrics = await lookupLyricaLyrics(track.name, artist, provider);
      setState(lyrics ? { kind: 'found', lyrics } : { kind: 'missing' });
    } catch (reason) {
      setState({ kind: 'error', message: lyricaError(reason) });
    }
  }

  if (lyrics) {
    const name = lyricaProviders.find((option) => option.value === lyrics.provider)?.label;
    return (
      <div className='lyrics-result'>
        <p className='lyrics-credit' role='status'>{name ? `Источник: ${name}` : 'Источник: Lyrica'}</p>
        {lyrics.instrumental && !lines.length && !lyrics.text.trim()
          ? <div className='lyrics-status'>Инструментальный трек</div>
          : <LyricsReader path={track.path} text={lyrics.text} lines={lines} />}
      </div>
    );
  }
  return (
    <div className='lyrics-status'>
      {state.kind === 'idle' && <p>{artist.trim() ? 'Для поиска Lyrica получит название трека и исполнителя.' : 'Укажите исполнителя в метаданных трека, чтобы найти текст.'}</p>}
      {state.kind === 'loading' && <p role='status'>Ищем текст…</p>}
      {state.kind === 'missing' && <p role='status'>Текст не найден. Можно попробовать другой источник.</p>}
      {state.kind === 'error' && <p className='kit-error' role='alert'>{state.message}</p>}
      <Button size='small' icon='Search' disabled={state.kind === 'loading' || !artist.trim()}
        onClick={() => void search()}>{state.kind === 'missing' || state.kind === 'error' ? 'Повторить поиск' : 'Найти текст'}</Button>
    </div>
  );
}

function Lyrics() {
  const track = usePlayerStore((state) => state.currentTrack);
  const nextPath = usePlayerStore((state) => state.nextTrackPath);
  const [nextTrack, setNextTrack] = useState<Track>();
  const [source, setSource] = useState<Source>('local');
  const path = track?.path;

  useEffect(() => {
    if (!nextPath || nextPath === path) return;
    let active = true;
    void tracksByPaths([nextPath]).then(([next]) => { if (active) setNextTrack(next); }).catch(() => undefined);
    return () => { active = false; };
  }, [nextPath, path]);

  if (!track) return <p className='kit-message'>Выберите трек, чтобы увидеть текст.</p>;
  const next = nextPath === path ? track : nextTrack?.path === nextPath ? nextTrack : undefined;
  return (
    <section className='collection-details lyrics-details'>
      <MediaDetails item={{ name: track.name, subtitle: track.artists.join(', '), icon: 'Track', path }}
        metadata={[
          ...(track.year ? [{ icon: 'Calendar' as const, label: String(track.year) }] : []),
          ...(track.genre ? [{ icon: 'Melody' as const, label: track.genre }] : []),
        ]}
        next={next && { name: next.name, subtitle: next.artists.join(', '), icon: 'Track', path: next.path }} />
      <div className='lyrics-panel'>
        <div className='lyrics-toolbar'>
          <label htmlFor='lyrics-source'>Источник текста</label>
          <Select id='lyrics-source' label='Источник текста' value={source} options={sources} onChange={setSource} />
        </div>
        {source === 'local' ? <LocalLyrics key={track.path} track={track} onSearch={() => setSource('auto')} />
          : source === 'geniusPage' ? <GeniusLookup key={track.path} track={track} />
            : <LyricaLookup key={`${source}:${track.path}`} track={track} provider={source} />}
      </div>
    </section>
  );
}
