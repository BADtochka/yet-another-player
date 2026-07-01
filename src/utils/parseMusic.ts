import { useMusicStore, useSettingsStore } from '@/stores';
import { registerMusicRoots } from '@/utils/audioServer';
import { getTracksFromFolders } from '@/utils/getTracks';

export const parseMusic = async (folders?: string[]) => {
  const musicFolders = folders ?? useSettingsStore.getState().musicFolders;
  const validFolders = [...new Set(musicFolders.map((folder) => folder.trim()).filter(Boolean))];

  if (validFolders.length === 0) {
    throw new Error('Music folders are not selected');
  }

  await registerMusicRoots(validFolders);

  const tracks = await getTracksFromFolders(validFolders);

  useMusicStore.getState().setTracks(tracks);
};
