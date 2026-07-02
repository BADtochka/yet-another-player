import { useMusicStore, useSettingsStore } from '@/stores';
import { registerMusicRoots } from '@/utils/audioServer';
import { getTracksFromFolders, type TrackScanProgress } from '@/utils/getTracks';

type ParseMusicOptions = {
  folders?: string[];
  onProgress?: (progress: TrackScanProgress) => void;
};

export const parseMusic = async (options?: string[] | ParseMusicOptions) => {
  const folders = Array.isArray(options) ? options : options?.folders;
  const musicFolders = folders ?? useSettingsStore.getState().musicFolders;
  const validFolders = [...new Set(musicFolders.map((folder) => folder.trim()).filter(Boolean))];

  if (validFolders.length === 0) {
    throw new Error('Music folders are not selected');
  }

  await registerMusicRoots(validFolders);

  const tracks = await getTracksFromFolders(validFolders, {
    onProgress: Array.isArray(options) ? undefined : options?.onProgress,
  });

  useMusicStore.getState().setTracks(tracks);
};
