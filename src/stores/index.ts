import { loadCatalog } from '@/utils/parseMusic';
import { startPlayer } from '@/stores/playerStore';
import { settingsStoreHandler, useSettingsStore } from '@/stores/settingsStore';
import { registerMusicRoots } from '@/utils/musicFolders';

let started = false;

const foldersChanged = (current: string[], previous: string[]) =>
  current.length !== previous.length || current.some((folder, index) => folder !== previous[index]);

export const startStores = async () => {
  if (started) {
    return;
  }

  started = true;

  await settingsStoreHandler.start();

  const { musicFolders, setMusicFolders } = useSettingsStore.getState();
  setMusicFolders(musicFolders);

  useSettingsStore.subscribe((state, prev) => {
    if (foldersChanged(state.musicFolders, prev.musicFolders)) {
      void registerMusicRoots(state.musicFolders).catch((error) => console.warn('Could not register music folders', error));
    }
  });

  await registerMusicRoots(useSettingsStore.getState().musicFolders);
  await loadCatalog();
  await startPlayer();
};

export { usePlayerStore } from '@/stores/playerStore';
export { useSettingsStore } from '@/stores/settingsStore';
