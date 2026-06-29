import { invoke } from '@tauri-apps/api/core';

let baseUrlPromise: Promise<string> | null = null;

const getBaseUrl = () => {
  if (!baseUrlPromise) {
    baseUrlPromise = invoke<string>('audio_server_base');
  }

  return baseUrlPromise;
};

export const registerMusicRoot = async (path: string) => {
  const musicPath = path.trim();

  if (!musicPath) {
    return;
  }

  await invoke('set_music_root', { path: musicPath });
};

export const warmupAudioServer = async () => {
  const baseUrl = await getBaseUrl();

  try {
    await fetch(`${baseUrl}/stream?path=${encodeURIComponent('__warmup__')}`);
  } catch {
    // Warmup is best-effort.
  }
};

export const getAudioSrc = async (path: string) => {
  const baseUrl = await getBaseUrl();

  return `${baseUrl}/stream?path=${encodeURIComponent(path)}`;
};
