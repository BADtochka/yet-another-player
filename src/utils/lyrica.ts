import { invoke } from '@tauri-apps/api/core';
import { geniusReady, lyricaReady, useSettingsStore } from '@/stores/settingsStore';
import { geniusLyrics, geniusPageUrl } from '@/utils/genius';
import type { LyricsSource } from '@/types/Settings';
import type { Track } from '@/types/Track';

export const lyricaProviders = [
  { value: 'auto', label: 'Автоматически' },
  { value: 'lrclib', label: 'LRCLIB' },
  { value: 'lrcmux', label: 'Lrcmux' },
  { value: 'videoCaptions', label: 'Субтитры видео' },
  { value: 'netease', label: 'NetEase' },
  { value: 'megalobiz', label: 'Megalobiz' },
  { value: 'musixmatch', label: 'Musixmatch' },
  { value: 'appleMusic', label: 'Apple Music' },
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

export function localLyricsText(track: Track, local: LocalLyricsData | null) {
  return [local?.sidecar, track.lyrics.join('\n'), local?.fetched]
    .find((text) => text?.trim() && !isSingleTimedLine(text));
}

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

export const readSynchronizedLyrics = ({ lines }: LyricaLyrics) => ({
  text: '',
  lines: lines.map(({ startMs, text }) => ({ time: startMs / 1000, content: text })),
});

export async function saveProviderLyrics(path: string, resolved: ReturnType<typeof readProviderLyrics>, expectedFetched?: string | null) {
  const text = resolved.lines.length ? resolved.lines.map(({ time, content }) => {
    const startMs = Math.round(time * 1000);
    const minutes = Math.floor(startMs / 60_000);
    const seconds = ((startMs % 60_000) / 1000).toFixed(3).padStart(6, '0');
    return `[${minutes}:${seconds}]${content.replace(/[\r\n]+/g, ' ')}`;
  }).join('\n') : resolved.text;
  if (text.trim() && !isSingleTimedLine(text)) {
    await invoke('save_track_lyrics', { path, text, background: expectedFetched !== undefined, expectedFetched, sidecar: resolved.lines.length > 0 });
  }
}

const automaticLookups = new Map<string, Promise<LyricaLyrics | null>>();

export function lookupAutomaticLyrics(track: Track) {
  const artist = track.artists.join(', ').trim();
  if (!artist) return Promise.resolve(null);
  const key = JSON.stringify([track.path, track.name, artist]);
  const pending = automaticLookups.get(key);
  if (pending) return pending;
  const request = (async () => {
    for (const provider of ['auto', 'lrclib', 'lrcmux'] as const) {
      const result = await lookupLyricaLyrics(track.name, artist, provider);
      if (!result || result.instrumental) {
        if (result?.instrumental) return result;
        continue;
      }
      const parsed = parseLrc(result.text);
      const lines = parsed.length > 1
        ? parsed.map(({ time, content }) => ({ startMs: Math.round(time * 1000), text: content }))
        : result.lines;
      if (lines.filter((line) => line.text.trim()).length > 1) return { ...result, lines };
    }
    return null;
  })().finally(() => automaticLookups.delete(key));
  automaticLookups.set(key, request);
  return request;
}

export function lyricaError(reason: unknown) {
  switch (reason) {
    case 'invalidTrack': return 'Для поиска в Lyrica нужны название трека и исполнитель длиной до 200 символов.';
    case 'timedOut': return 'Lyrica не успела ответить. Попробуйте ещё раз или выберите другой источник.';
    case 'rateLimited': return 'Lyrica ограничила число запросов. Попробуйте чуть позже.';
    case 'notInstalled': return 'Установите локальную Lyrica в настройках.';
    case 'localUnavailable': return 'Не удалось запустить локальную Lyrica. Проверьте её в настройках инструментов.';
    case 'network': return 'Локальная Lyrica не отвечает. Повторите поиск.';
    default: return 'Lyrica не смогла обработать запрос. Попробуйте чуть позже.';
  }
}


export type LyricsResult = {
  label: string;
  instrumental: boolean;
  content: ReturnType<typeof readProviderLyrics>;
};

export function canSearchLyrics(settings: ReturnType<typeof useSettingsStore.getState>) {
  return settings.lyricsSource !== 'local' && (lyricaReady(settings) || geniusReady(settings));
}

export async function lookupLyrics(track: Track, provider: Exclude<LyricsSource, 'local'>): Promise<LyricsResult | null> {
  if (provider === 'genius') {
    const text = await geniusLyrics(track.name, track.artists);
    return text ? { label: 'Genius', instrumental: false, content: { text, lines: [] } } : null;
  }
  const result = await (provider === 'auto'
    ? lookupAutomaticLyrics(track) : lookupLyricaLyrics(track.name, track.artists.join(', '), provider));
  if (!result) return null;
  return {
    label: lyricaProviders.find((option) => option.value === result.provider)?.label ?? 'Lyrica',
    instrumental: result.instrumental,
    content: provider === 'auto' ? readSynchronizedLyrics(result) : readProviderLyrics(result),
  };
}

const preferredLookups = new Map<string, Promise<LyricsResult | null>>();

export function lookupPreferredLyrics(track: Track) {
  const settings = useSettingsStore.getState();
  if (!canSearchLyrics(settings) || !track.artists.join('').trim()) return Promise.resolve(null);
  const order = [...new Set([settings.lyricsSource, 'auto', 'genius'] as const)]
    .filter((source): source is Exclude<LyricsSource, 'local'> => source !== 'local'
      && (source === 'genius' ? geniusReady(settings) : lyricaReady(settings)));
  const key = JSON.stringify([track.path, track.name, track.artists, order]);
  const active = preferredLookups.get(key);
  if (active) return active;

  const request = (async () => {
    let failure: { reason: unknown; provider: 'genius' | 'lyrica' } | undefined;
    for (const provider of order) {
      const current = useSettingsStore.getState();
      if (current.lyricsSource !== settings.lyricsSource || !canSearchLyrics(current)) return null;
      if (provider === 'genius' ? !geniusReady(current) : !lyricaReady(current)) continue;
      try {
        const result = await lookupLyrics(track, provider);
        const latest = useSettingsStore.getState();
        if (latest.lyricsSource !== settings.lyricsSource) return null;
        if (result && (provider === 'genius' ? geniusReady(latest) : lyricaReady(latest))) return result;
      } catch (reason) {
        const error: NonNullable<typeof failure> = { reason, provider: provider === 'genius' ? 'genius' : 'lyrica' };
        if (provider === 'genius' && geniusPageUrl(reason)) failure = error;
        else failure ??= error;
      }
    }
    if (failure) throw failure;
    return null;
  })().finally(() => preferredLookups.delete(key));
  preferredLookups.set(key, request);
  return request;
}
