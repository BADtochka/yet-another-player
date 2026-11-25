import { loadCatalog } from '@/utils/parseMusic';
import { startPlayer } from '@/stores/playerStore';
import { settingsStoreHandler, useSettingsStore } from '@/stores/settingsStore';
import { registerMusicRoots } from '@/utils/musicFolders';
import { getExternalToolStatus, loadDisabledTools } from '@/utils/externalTools';
import { geniusIsConfigured } from '@/utils/genius';

let started = false;

const foldersChanged = (current: string[], previous: string[]) =>
  current.length !== previous.length || current.some((folder, index) => folder !== previous[index]);

export const startStores = async () => {
  if (started) {
    return;
  }

  started = true;

  await settingsStoreHandler.start();
  await loadDisabledTools().catch((error) => console.warn('Could not read external tool settings', error));
  void geniusIsConfigured().catch(() => undefined);
  const lyricaStatus = getExternalToolStatus('lyrica').catch((error) => console.warn('Could not check local Lyrica', error));

  const { musicFolders, setMusicFolders } = useSettingsStore.getState();
  setMusicFolders(musicFolders);

  useSettingsStore.subscribe((state, prev) => {
    if (foldersChanged(state.musicFolders, prev.musicFolders)) {
      void registerMusicRoots(state.musicFolders).catch((error) => console.warn('Could not register music folders', error));
    }
  });

  await registerMusicRoots(useSettingsStore.getState().musicFolders);
  await Promise.all([loadCatalog(), lyricaStatus]);
  await startPlayer();
};

export { usePlayerStore } from '@/stores/playerStore';
export { useSettingsStore } from '@/stores/settingsStore';
