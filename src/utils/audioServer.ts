import { invoke } from '@tauri-apps/api/core';

const FALLBACK_COVER = '/temp_cover.png';

let baseUrlPromise: Promise<string> | null = null;
const coverSrcCache = new Map<string, string | null>();

const getBaseUrl = () => {
  if (!baseUrlPromise) {
    baseUrlPromise = invoke<string>('audio_server_base');
  }

  return baseUrlPromise;
};

export const registerMusicRoots = async (paths: string[]) => {
  const roots = paths.map((path) => path.trim()).filter(Boolean);

  await invoke('set_music_roots', { paths: roots });
};

export const warmupAudioServer = async () => {
  const baseUrl = await getBaseUrl();

  try {
    await fetch(`${baseUrl}/warmup`, { method: 'HEAD' });
  } catch {
    // Warmup is best-effort.
  }
};

export const getCoverSrc = async (path: string) => {
  if (coverSrcCache.has(path)) {
    return coverSrcCache.get(path) ?? FALLBACK_COVER;
  }

  try {
    const url = await invoke<string | null>('get_cover_src', { path });
    coverSrcCache.set(path, url);
    return url ?? FALLBACK_COVER;
  } catch {
    coverSrcCache.set(path, null);
    return FALLBACK_COVER;
  }
};

export const getAudioSrc = async (path: string) => {
  const baseUrl = await getBaseUrl();

  return `${baseUrl}/stream?path=${encodeURIComponent(path)}`;
};
