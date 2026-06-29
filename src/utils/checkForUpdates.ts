import { relaunch } from '@tauri-apps/plugin-process';
import { check } from '@tauri-apps/plugin-updater';

export const checkForAppUpdates = async () => {
  if (!import.meta.env.PROD) {
    return;
  }

  try {
    const update = await check();
    if (!update) {
      return;
    }

    await update.downloadAndInstall();
    await relaunch();
  } catch (error) {
    console.error('Failed to check or install update', error);
  }
};
