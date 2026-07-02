import { musicStoreHandler, useMusicStore } from '@/stores/musicStore';
import {
  loadPersistedPlaybackPosition,
  playerStoreHandler,
  restorePersistedPlayerState,
} from '@/stores/playerStore';
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

  const { musicFolders, setMusicFolders } = useSettingsStore.getState();
  setMusicFolders(musicFolders);

  await musicStoreHandler.start();

  const { tracks, setTracks } = useMusicStore.getState();
  setTracks(tracks);

  await playerStoreHandler.start();
  await loadPersistedPlaybackPosition();
  restorePersistedPlayerState();

  useMusicStore.subscribe((_state, prev) => {
    if (_state.tracks === prev.tracks) {
      return;
    }

    restorePersistedPlayerState();
  });

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
