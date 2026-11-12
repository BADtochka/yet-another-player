import { invoke } from '@tauri-apps/api/core';

export const lyricaProviders = [
  { value: 'auto', label: 'Lyrica · автоматически' },
  { value: 'lrclib', label: 'LRCLIB' },
  { value: 'lrcmux', label: 'Lrcmux' },
  { value: 'youtube', label: 'YouTube Music' },
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

export const lookupLyricaLyrics = (title: string, artist: string, provider: LyricaProvider) =>
  invoke<LyricaLyrics | null>('lookup_lyrica_lyrics', { title, artist, provider });

export function lyricaError(reason: unknown) {
  switch (reason) {
    case 'invalidTrack': return 'Для поиска в Lyrica нужны название трека и исполнитель длиной до 200 символов.';
    case 'timedOut': return 'Lyrica не успела ответить. Попробуйте ещё раз или выберите другой источник.';
    case 'rateLimited': return 'Lyrica ограничила число запросов. Попробуйте чуть позже.';
    case 'network': return 'Не удалось подключиться к Lyrica. Проверьте интернет и повторите поиск.';
    default: return 'Lyrica не смогла обработать запрос. Попробуйте чуть позже.';
  }
}
