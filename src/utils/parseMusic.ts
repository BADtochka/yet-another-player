import { load } from '@tauri-apps/plugin-store';
import { registerMusicRoot } from '@/utils/audioServer';
import { getTracks } from '@/utils/getTracks';

export const parseMusic = async (path?: string) => {
  const musicStore = await load('music.json', {
    defaults: {
      tracks: [],
    },
  });
  const settingsStore = await load('settings.json', {
    defaults: {
      musicPath: '',
    },
  });

  const defaultFolder = await settingsStore?.get<string>('musicPath');
  const musicPath = (path || defaultFolder || '').trim();

  if (!musicPath) {
    throw new Error('Music folder is not selected');
  }

  await registerMusicRoot(musicPath);

  const tracks = await getTracks(musicPath);

  await musicStore.set('tracks', tracks);
  await musicStore.save();

  if (path) {
    await settingsStore.set('musicPath', path);
    await settingsStore.save();
  }
};
