import { mockIPC } from '@tauri-apps/api/mocks';
import { afterAll, beforeEach, expect, test } from 'bun:test';
import type { Track } from '@/types/Track';
import type { LyricaLyrics, LyricaProvider } from '@/utils/lyrica';

Object.defineProperty(globalThis, 'window', { value: {}, configurable: true });
mockIPC(() => undefined);
const { useSettingsStore } = await import('@/stores/settingsStore');
const { localLyricsText, lookupPreferredLyrics, onlineLyricsSearch, saveProviderLyrics } = await import('@/utils/lyrica');

const track: Track = {
  id: 1, path: '/music/song.flac', name: 'Song', artists: ['Artist'], lyrics: [],
  album: null, albumId: '', albumArtist: null, discNumber: null, trackNumber: null,
  genre: null, genres: [], year: null, releaseDate: null, playCount: 0, duration: null, replayGainDb: null,
};
const plainText = 'First\nSecond';
const lrc = '[00:01.00]First\n[00:03.00]Second';
const lrclib: LyricaLyrics = { provider: 'lrclib', instrumental: false, text: lrc, lines: [] };
const requested: string[] = [];

function mockProviders(lookup: (provider: LyricaProvider) => LyricaLyrics | null) {
  mockIPC((command, args) => {
    if (command === 'lookup_lyrica_lyrics') {
      const { provider } = args as { provider: LyricaProvider };
      requested.push(provider);
      return lookup(provider);
    }
    if (command === 'genius_lyrics') {
      requested.push('genius');
      return plainText;
    }
  });
}

beforeEach(() => {
  requested.length = 0;
  useSettingsStore.setState({ lyricsSource: 'auto', lyricaInstalled: true, lyricaEnabled: true, geniusConfigured: true });
});

afterAll(() => {
  Reflect.deleteProperty(globalThis, 'window');
});

test('automatic search uses available LRCLIB timestamps before querying Genius', async () => {
  mockProviders((provider) => provider === 'lrclib' ? lrclib : null);

  const result = await lookupPreferredLyrics(track);

  expect(result?.content.lines).toEqual([{ time: 1, content: 'First' }, { time: 3, content: 'Second' }]);
  expect(requested).toEqual(['lrclib']);
});

test('an aggregate lookup timeout still allows a synchronized result from Lrcmux', async () => {
  mockProviders((provider) => {
    if (provider === 'auto') throw 'timedOut';
    return provider === 'lrcmux' ? { ...lrclib, provider: 'lrcmux' } : null;
  });

  expect((await lookupPreferredLyrics(track))?.label).toBe('Lrcmux');
  expect(requested).toEqual(['lrclib', 'auto', 'lrcmux']);
});

test('Genius is the fallback after the synchronized sources have no timestamps', async () => {
  mockProviders(() => ({ ...lrclib, text: plainText }));

  expect((await lookupPreferredLyrics(track))?.label).toBe('Genius');
  expect(requested).toEqual(['lrclib', 'auto', 'lrcmux', 'genius']);
});

test('an in-flight Genius fallback yields when Lyrica becomes available', async () => {
  useSettingsStore.setState({ lyricaInstalled: false });
  mockIPC((command) => {
    if (command === 'genius_lyrics') {
      useSettingsStore.setState({ lyricaInstalled: true });
      return plainText;
    }
  });

  expect(await lookupPreferredLyrics(track)).toBeNull();
});

test('cached plain text remains the fallback when Lyrica has no synchronized text', async () => {
  mockProviders(() => null);
  const local = { sidecar: null, fetched: 'First\r\nSecond\r\n' };
  const text = localLyricsText({ ...track, lyrics: ['First', 'Second'] }, local, true);
  const search = onlineLyricsSearch(useSettingsStore.getState(), text, local.fetched);

  expect(search).toBe('synchronized');
  expect(await lookupPreferredLyrics(track, search)).toBeNull();
  expect(requested).toEqual(['lrclib', 'auto', 'lrcmux']);
});

test('synchronized lyrics replace the cached text using the original cache value', async () => {
  let saved: unknown;
  mockIPC((command, args) => {
    if (command === 'lookup_lyrica_lyrics') return lrclib;
    if (command === 'save_track_lyrics') saved = args;
  });
  const search = onlineLyricsSearch(useSettingsStore.getState(), plainText, plainText);
  const result = await lookupPreferredLyrics(track, search);

  if (result) await saveProviderLyrics(track.path, result.content, plainText);

  expect(saved).toEqual({
    path: track.path, text: '[0:01.000]First\n[0:03.000]Second',
    background: true, expectedFetched: plainText, sidecar: true,
  });
});

test('automatic mode reads cached timestamps before embedded plain text', () => {
  const local = { sidecar: null, fetched: lrc };
  const tagged = { ...track, lyrics: [plainText] };

  expect(localLyricsText(tagged, local, true)).toBe(lrc);
  expect(localLyricsText(tagged, local)).toBe(plainText);
});

test('a configured Genius preference still applies to new online searches', async () => {
  mockProviders(() => lrclib);
  useSettingsStore.setState({ lyricsSource: 'genius' });

  expect((await lookupPreferredLyrics(track))?.label).toBe('Genius');
  expect(requested).toEqual(['genius']);
});
