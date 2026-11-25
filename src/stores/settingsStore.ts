import { type Settings, type ToolName, type TransitionSettings } from '@/types/Settings';
import { createTauriStore } from '@tauri-store/zustand';
import { create } from 'zustand';

type SettingsState = Settings & {
  lyricaInstalled: boolean;
  geniusConfigured: boolean;
  disabledTools: ToolName[];
  setShowAudioInfo: (enabled: boolean) => void;
  setSeparateVideoLyrics: (enabled: boolean) => void;
  setSuspendHiddenVideo: (enabled: boolean) => void;
  setLyricaEnabled: (enabled: boolean) => void;
  setResolver: (config: Pick<Settings, 'resolverMode' | 'resolverUrl' | 'resolverSources'>) => void;
  addMusicFolder: (path: string) => void;
  removeMusicFolder: (path: string) => void;
  setMusicFolders: (folders: string[]) => void;
  setVolumeNormLevel: (level: Settings['volumeNormLevel']) => void;
  setVolumeNormEnabled: (enabled: Settings['volumeNormEnabled']) => void;
  setVolumeGesture: (binding: Settings['volumeGesture']) => void;
  setVolumeGestureEnabled: (enabled: boolean) => void;
  setRestartOnPrevious: (enabled: boolean) => void;
  setTransitions: (settings: Partial<TransitionSettings>) => void;
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
  showAudioInfo: true,
  setShowAudioInfo: (showAudioInfo) => set({ showAudioInfo }),
  separateVideoLyrics: false,
  setSeparateVideoLyrics: (separateVideoLyrics) => set({ separateVideoLyrics }),
  suspendHiddenVideo: true,
  setSuspendHiddenVideo: (suspendHiddenVideo) => set({ suspendHiddenVideo }),
  musicFolders: [],
  lyricaEnabled: false,
  lyricaInstalled: false,
  geniusConfigured: false,
  disabledTools: [],
  metadataSource: 'auto',
  showExternalAlbumTracks: true,
  lyricsSource: 'auto',
  setLyricaEnabled: (lyricaEnabled) => set({ lyricaEnabled }),
  resolverUrl: '',
  resolverSources: [],
  setResolver: (config) => set({ ...config, resolverUrl: config.resolverUrl.trim() }),
  volumeNormEnabled: false,
  volumeNormLevel: 'normal',
  volumeGesture: null,
  setVolumeGesture: (volumeGesture) => set((state) => ({
    volumeGesture,
    volumeGestureEnabled: volumeGesture && !state.volumeGesture ? true : state.volumeGestureEnabled,
  })),
  setVolumeGestureEnabled: (volumeGestureEnabled) => set({ volumeGestureEnabled }),
  restartOnPrevious: true,
  setRestartOnPrevious: (restartOnPrevious) => set({ restartOnPrevious }),
  transitions: {
    enabled: false,
    mode: 'smooth',
    seconds: 5,
    maxSeconds: 8,
    preset: 'auto',
    intensity: 0.7,
    trimSilence: true,
    alignBeats: true,
  },
  setTransitions: (settings) => set((state) => ({ transitions: { ...state.transitions, ...settings } })),
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

export const lyricaReady = (settings: SettingsState) => settings.lyricaInstalled && settings.lyricaEnabled && !settings.disabledTools.includes('lyrica');

export const geniusReady = (settings: SettingsState) => settings.geniusConfigured && !settings.disabledTools.includes('genius');

export const settingsStoreHandler = createTauriStore('settings', useSettingsStore, {
  saveOnChange: true,
  filterKeys: ['showAudioInfo', 'separateVideoLyrics', 'suspendHiddenVideo', 'lyricaEnabled', 'musicFolders', 'volumeNormLevel', 'volumeNormEnabled', 'volumeGesture', 'volumeGestureEnabled', 'restartOnPrevious', 'resolverUrl', 'resolverMode', 'resolverSources', 'metadataSource', 'showExternalAlbumTracks', 'lyricsSource', 'transitions'],
  filterKeysStrategy: 'pick',
});

export const metadataResolverConfig = () => {
  const { resolverMode, resolverUrl, resolverSources, metadataSource, disabledTools } = useSettingsStore.getState();
  return {
    mode: disabledTools.includes('ommr') ? 'off' : resolverMode ?? (resolverUrl ? 'remote' : 'off'),
    url: resolverUrl || null,
    sources: resolverSources,
    primary: metadataSource,
  };
};
