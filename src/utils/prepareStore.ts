import { startStores, useSettingsStore } from '@/stores';
import { warmupAudioServer } from '@/utils/audioServer';
import { warmupPlayback } from '@/utils/warmupPlayback';

export const prepareStore = async () => {
  await startStores();

  const hasMusicFolders = useSettingsStore.getState().musicFolders.length > 0;

  void warmupAudioServer();
  void warmupPlayback();

  return { hasMusicFolders };
};
