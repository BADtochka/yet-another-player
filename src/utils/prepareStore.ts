import { startStores, useSettingsStore } from '@/stores';

export const prepareStore = async () => {
  await startStores();

  const hasMusicFolders = useSettingsStore.getState().musicFolders.length > 0;

  return { hasMusicFolders };
};
