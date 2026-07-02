import { useMusicStore, useSettingsStore } from '@/stores';
import { registerMusicRoots } from '@/utils/audioServer';
import {
  collectAudioPathsFromFolders,
  createTrackStub,
  parseTracksInBatches,
  type TrackScanProgress,
} from '@/utils/getTracks';

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

  const onProgress = Array.isArray(options) ? undefined : options?.onProgress;
  const paths = await collectAudioPathsFromFolders(validFolders, { onProgress });
  const store = useMusicStore.getState();

  store.setTracks(paths.map(createTrackStub));

  await parseTracksInBatches(paths, {
    onProgress,
    onBatch: (tracks, progress) => {
      store.mergeTracks(tracks);
      onProgress?.(progress);
    },
  });
};
