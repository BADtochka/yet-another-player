import { invoke } from '@tauri-apps/api/core';

const FALLBACK_COVER = '/temp_cover.png';

let baseUrlPromise: Promise<string> | null = null;

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
    await fetch(`${baseUrl}/stream?path=${encodeURIComponent('__warmup__')}`);
  } catch {
    // Warmup is best-effort.
  }
};

export const getCoverSrc = async (path: string) => {
  const baseUrl = await getBaseUrl();
  const url = `${baseUrl}/cover?path=${encodeURIComponent(path)}`;

  try {
    const response = await fetch(url, { method: 'HEAD' });

    if (response.ok) {
      return url;
    }
  } catch {
    // Fall back to the placeholder cover.
  }

  return FALLBACK_COVER;
};

export const getAudioSrc = async (path: string) => {
  const baseUrl = await getBaseUrl();

  return `${baseUrl}/stream?path=${encodeURIComponent(path)}`;
};
