import { createTauriStore } from '@tauri-store/zustand';
import { create } from 'zustand';
import { type Track } from '@/types/Track';

type MusicState = {
  tracks: Track[];
  setTracks: (tracks: Track[]) => void;
  mergeTracks: (tracks: Track[]) => void;
  addTrack: (track: Track) => void;
};

const stripLegacyCover = (tracks: Array<Track & { coverBase64?: string; replayGainDb?: number | null }>): Track[] =>
  tracks.map(({ coverBase64: _cover, replayGainDb, ...track }) => ({
    ...track,
    replayGainDb: replayGainDb ?? null,
  }));

export const useMusicStore = create<MusicState>((set) => ({
  tracks: [],
  setTracks: (tracks) => set({ tracks: stripLegacyCover(tracks) }),
  mergeTracks: (updates) =>
    set((state) => {
      if (updates.length === 0) {
        return state;
      }

      const updateMap = new Map(updates.map((track) => [track.path, track]));

      return {
        tracks: state.tracks.map((track) => updateMap.get(track.path) ?? track),
      };
    }),
  addTrack: (track) =>
    set((state) => ({
      tracks: [...state.tracks.filter((existingTrack) => existingTrack.path !== track.path), track].sort((a, b) =>
        a.path.localeCompare(b.path),
      ),
    })),
}));

export const musicStoreHandler = createTauriStore('music', useMusicStore, {
  saveOnChange: true,
  saveStrategy: 'debounce',
  saveInterval: 300,
  filterKeys: ['tracks'],
  filterKeysStrategy: 'pick',
});
