import { type Settings } from '@/types/Settings';
import { createTauriStore } from '@tauri-store/zustand';
import { create } from 'zustand';

type SettingsState = Settings & {
  addMusicFolder: (path: string) => void;
  removeMusicFolder: (path: string) => void;
  setMusicFolders: (folders: string[]) => void;
  setVolumeNormLevel: (level: Settings['volumeNormLevel']) => void;
  setVolumeNormEnabled: (enabled: Settings['volumeNormEnabled']) => void;
};

const normalizeFolder = (path: string) => path.trim();

const normalizeFolders = (folders: string[]) => {
  const unique = new Set<string>();

  for (const folder of folders) {
    const normalized = normalizeFolder(folder);

    if (normalized) {
      unique.add(normalized);
    }
  }

  return [...unique];
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  musicFolders: [],
  volumeNormEnabled: false,
  volumeNormLevel: 'normal',
  addMusicFolder: (path) => {
    const normalized = normalizeFolder(path);

    if (!normalized) {
      return;
    }

    set({ musicFolders: normalizeFolders([...get().musicFolders, normalized]) });
  },
  removeMusicFolder: (path) => {
    const normalized = normalizeFolder(path);
    set({ musicFolders: get().musicFolders.filter((folder) => folder !== normalized) });
  },
  setMusicFolders: (folders) => {
    set({ musicFolders: normalizeFolders(folders) });
  },
  setVolumeNormEnabled: (enabled) => set({ volumeNormEnabled: enabled }),
  setVolumeNormLevel: (volumeNormLevel) => set({ volumeNormLevel }),
}));

export const settingsStoreHandler = createTauriStore('settings', useSettingsStore, {
  saveOnChange: true,
  filterKeys: ['musicFolders', 'volumeNormLevel', 'volumeNormEnabled'],
  filterKeysStrategy: 'pick',
});
