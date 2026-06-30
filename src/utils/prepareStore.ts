import { load } from '@tauri-apps/plugin-store';
import { registerMusicRoot, warmupAudioServer } from '@/utils/audioServer';
import { warmupPlayback } from '@/utils/warmupPlayback';

export const prepareStore = async () => {
  const settings = await load('settings.json', {
    defaults: {
      musicPath: '',
    },
  });
  const music = await load('music.json', {
    defaults: {
      tracks: [],
    },
  });
  const musicPath = (await settings.get<string>('musicPath'))?.trim() ?? '';

  await registerMusicRoot(musicPath);
  void warmupAudioServer();
  void warmupPlayback();

  return { settings, music, musicPath };
};
