import { invoke } from '@tauri-apps/api/core';

export const registerMusicRoots = (paths: string[]) => invoke('set_music_roots', { paths });
