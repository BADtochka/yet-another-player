import { create } from 'zustand';
import { useMusicStore } from '@/stores/musicStore';
import { type Track } from '@/types/Track';
import { getNextTrack, getPreviousTrack } from '@/utils/playQueue';

type PlayerState = {
  volume: number;
  currentTrack: Track | null;
  playing: boolean;
  seeking: boolean;
  currentTime: number;
  duration: number;
  setVolume: (volume: number) => void;
  setCurrentTrack: (track: Track | null) => void;
  setPlaying: (playing: boolean) => void;
  setSeeking: (seeking: boolean) => void;
  setCurrentTime: (currentTime: number) => void;
  setDuration: (duration: number) => void;
  playNext: () => void;
  playPrevious: () => void;
};

export const usePlayerStore = create<PlayerState>((set, get) => ({
  volume: 0.1,
  currentTrack: null,
  playing: false,
  seeking: false,
  currentTime: 0,
  duration: 0,
  setVolume: (volume) => set({ volume }),
  setCurrentTrack: (currentTrack) => set({ currentTrack }),
  setPlaying: (playing) => set({ playing }),
  setSeeking: (seeking) => set({ seeking }),
  setCurrentTime: (currentTime) => set({ currentTime }),
  setDuration: (duration) => set({ duration }),
  playNext: () => {
    const { currentTrack } = get();
    const tracks = useMusicStore.getState().tracks;
    const next = getNextTrack(tracks, currentTrack);

    if (!next) {
      set({ playing: false, currentTime: 0 });
      return;
    }

    set({ currentTrack: next, currentTime: 0, playing: true });
  },
  playPrevious: () => {
    const { currentTrack } = get();
    const tracks = useMusicStore.getState().tracks;
    const previous = getPreviousTrack(tracks, currentTrack);

    if (!previous) {
      set({ playing: false, currentTime: 0 });
      return;
    }

    set({ currentTrack: previous, currentTime: 0, playing: true });
  },
}));
