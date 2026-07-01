import { createTauriStore } from '@tauri-store/zustand';
import { create } from 'zustand';
import { type Track } from '@/types/Track';

type MusicState = {
  tracks: Track[];
  setTracks: (tracks: Track[]) => void;
};

const stripLegacyCover = (tracks: Array<Track & { coverBase64?: string }>): Track[] =>
  tracks.map(({ coverBase64: _cover, ...track }) => track);

export const useMusicStore = create<MusicState>((set) => ({
  tracks: [],
  setTracks: (tracks) => set({ tracks: stripLegacyCover(tracks) }),
}));

export const musicStoreHandler = createTauriStore('music', useMusicStore, {
  saveOnChange: true,
  saveStrategy: 'debounce',
  saveInterval: 300,
  filterKeys: ['tracks'],
  filterKeysStrategy: 'pick',
  hooks: {
    beforeFrontendSync: (state) => ({
      tracks: stripLegacyCover((state.tracks as Array<Track & { coverBase64?: string }>) ?? []),
    }),
  },
});
