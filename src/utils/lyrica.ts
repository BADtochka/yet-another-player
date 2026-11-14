import { invoke } from '@tauri-apps/api/core';

export const lyricaProviders = [
  { value: 'auto', label: 'Lyrica · автоматически' },
  { value: 'lrclib', label: 'LRCLIB' },
  { value: 'lrcmux', label: 'Lrcmux' },
  { value: 'videoCaptions', label: 'Субтитры видео' },
  { value: 'netease', label: 'NetEase' },
  { value: 'megalobiz', label: 'Megalobiz' },
  { value: 'musixmatch', label: 'Musixmatch' },
  { value: 'appleMusic', label: 'Apple Music' },
  { value: 'genius', label: 'Genius · через Lyrica' },
] as const;

export type LyricaProvider = typeof lyricaProviders[number]['value'];

export interface LyricaLyrics {
  provider: Exclude<LyricaProvider, 'auto'> | null;
  instrumental: boolean;
  text: string;
  lines: { startMs: number; text: string }[];
}

const pendingLyrics = new Map<string, Promise<LyricaLyrics | null>>();

export function lookupLyricaLyrics(title: string, artist: string, provider: LyricaProvider) {
  const key = JSON.stringify([title, artist, provider]);
  const existing = pendingLyrics.get(key);
  if (existing) return existing;

  const request = invoke<LyricaLyrics | null>('lookup_lyrica_lyrics', { title, artist, provider })
    .finally(() => pendingLyrics.delete(key));
  pendingLyrics.set(key, request);
  return request;
}

export type LocalLyricsData = { sidecar: string | null; fetched: string | null };
export const getLocalLyrics = (path: string) => invoke<LocalLyricsData | null>('get_lyrics', { path });

export function parseLrc(text: string) {
  const offset = Number(text.match(/^\[offset:\s*([+-]?\d+)\]/im)?.[1] ?? 0) / 1000;
  return text.split(/\r\n?|\n/).flatMap((rawLine) => {
    const line = rawLine.trimStart();
    const prefix = line.match(/^(?:\[\d+:\d{2}(?:[.:]\d+)?\])+/)?.[0];
    if (!prefix) return [];
    const content = line.slice(prefix.length).trim();
    return Array.from(prefix.matchAll(/\[(\d+):(\d{2})(?:[.:](\d+))?\]/g), ([, minutes, seconds, fraction]) => ({
      time: Math.max(0, Number(minutes) * 60 + Number(seconds) + Number(`0.${fraction ?? '0'}`) + offset),
      content,
    }));
  }).sort((a, b) => a.time - b.time);
}

export function isSingleTimedLine(text: string) {
  const trimmed = text.trim();
  return !/[\r\n]/.test(trimmed) && parseLrc(trimmed).length === 1;
}

export function readProviderLyrics(lyrics: LyricaLyrics) {
  const { text } = lyrics;
  const timed = lyrics.lines.map((line) => ({ time: line.startMs / 1000, content: line.text }));
  const parsed = parseLrc(text);
  const timedCount = timed.filter((line) => line.content.trim()).length;
  if (parsed.length > 0 && parsed.filter((line) => line.content).length >= timedCount) {
    return { text, lines: parsed };
  }
  if (!parsed.length && text.split(/\r\n?|\n/).filter((line) => line.trim()).length > timedCount) {
    return { text, lines: [] };
  }
  return { text, lines: timed };
}

export async function saveProviderLyrics(path: string, resolved: ReturnType<typeof readProviderLyrics>, expectedFetched?: string | null) {
  const text = resolved.lines.length ? resolved.lines.map(({ time, content }) => {
    const startMs = Math.round(time * 1000);
    const minutes = Math.floor(startMs / 60_000);
    const seconds = ((startMs % 60_000) / 1000).toFixed(3).padStart(6, '0');
    return `[${minutes}:${seconds}]${content.replace(/[\r\n]+/g, ' ')}`;
  }).join('\n') : resolved.text;
  if (text.trim() && !isSingleTimedLine(text)) {
    await invoke('save_track_lyrics', { path, text, background: expectedFetched !== undefined, expectedFetched });
  }
}

export function lyricaError(reason: unknown) {
  switch (reason) {
    case 'invalidTrack': return 'Для поиска в Lyrica нужны название трека и исполнитель длиной до 200 символов.';
    case 'timedOut': return 'Lyrica не успела ответить. Попробуйте ещё раз или выберите другой источник.';
    case 'rateLimited': return 'Lyrica ограничила число запросов. Попробуйте чуть позже.';
    case 'network': return 'Не удалось подключиться к Lyrica. Проверьте интернет и повторите поиск.';
    default: return 'Lyrica не смогла обработать запрос. Попробуйте чуть позже.';
  }
}
