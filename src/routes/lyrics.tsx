import { createFileRoute } from '@tanstack/react-router';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { MediaDetails } from '@/components/MediaDetails';
import type { Track } from '@/types/Track';
import { tracksByPaths } from '@/hooks/useLibrary';
import { usePlayerStore } from '@/stores';

export const Route = createFileRoute('/lyrics')({ component: Lyrics });

type Line = { time: number; content: string };

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

function LocalLyrics({ track }: { track: Track }) {
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
    </div>
  );
}

function Lyrics() {
  const track = usePlayerStore((state) => state.currentTrack);
  const nextPath = usePlayerStore((state) => state.nextTrackPath);
  const [nextTrack, setNextTrack] = useState<Track>();
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
        <LocalLyrics key={track.path} track={track} />
      </div>
    </section>
  );
}
