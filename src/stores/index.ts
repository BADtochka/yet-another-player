import { musicStoreHandler } from '@/stores/musicStore';
import { settingsStoreHandler, useSettingsStore } from '@/stores/settingsStore';
import { registerMusicRoots } from '@/utils/audioServer';

let started = false;

const foldersChanged = (current: string[], previous: string[]) =>
  current.length !== previous.length || current.some((folder, index) => folder !== previous[index]);

export const startStores = async () => {
  if (started) {
    return;
  }

  started = true;

  await settingsStoreHandler.start();
  await musicStoreHandler.start();

  useSettingsStore.subscribe((state, prev) => {
    if (foldersChanged(state.musicFolders, prev.musicFolders)) {
      void registerMusicRoots(state.musicFolders);
    }
  });

  await registerMusicRoots(useSettingsStore.getState().musicFolders);
};

export { useMusicStore } from '@/stores/musicStore';
export { usePlayerStore } from '@/stores/playerStore';
export { useSettingsStore } from '@/stores/settingsStore';
