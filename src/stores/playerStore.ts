import { createTauriStore, getStoreState, saveNow } from '@tauri-store/zustand';
import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import { useMusicStore } from '@/stores/musicStore';
import { type Track } from '@/types/Track';
import { getNextTrack, getPreviousTrack } from '@/utils/playQueue';

type PlayerState = {
  volume: number;
  currentTrack: Track | null;
  currentTrackPath: string | null;
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

const clampVolume = (volume: number) => {
  if (!Number.isFinite(volume)) {
    return 0.1;
  }

  return Math.min(1, Math.max(0, volume));
};

const PLAYER_STORE_ID = 'player';

export const loadPersistedPlaybackPosition = async () => {
  const saved = await getStoreState<{ currentTime?: unknown }>(PLAYER_STORE_ID);

  if (typeof saved.currentTime === 'number' && Number.isFinite(saved.currentTime) && saved.currentTime > 0) {
    usePlayerStore.getState().setCurrentTime(saved.currentTime);
  }
};

export const savePlayerStateOnExit = async () => {
  const { volume, currentTrackPath, currentTime } = usePlayerStore.getState();

  await invoke('plugin:zustand|patch', {
    id: PLAYER_STORE_ID,
    state: {
      volume,
      currentTrackPath,
      currentTime,
    },
  });
  await saveNow(PLAYER_STORE_ID);
};

export const restorePersistedPlayerState = () => {
  const player = usePlayerStore.getState();
  const tracks = useMusicStore.getState().tracks;
  const volume = clampVolume(player.volume);

  if (volume !== player.volume) {
    player.setVolume(volume);
  }

  if (!player.currentTrackPath) {
    if (player.currentTrack) {
      player.setCurrentTrack(null);
    }

    return;
  }

  const track = tracks.find((item) => item.path === player.currentTrackPath) ?? null;

  player.setCurrentTrack(track);

  if (!track) {
    player.setCurrentTime(0);
  }
};

export const usePlayerStore = create<PlayerState>((set, get) => ({
  volume: 0.1,
  currentTrack: null,
  currentTrackPath: null,
  playing: false,
  seeking: false,
  currentTime: 0,
  duration: 0,
  setVolume: (volume) => set({ volume: clampVolume(volume) }),
  setCurrentTrack: (currentTrack) =>
    set({
      currentTrack,
      currentTrackPath: currentTrack?.path ?? null,
    }),
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

    set({ currentTrack: next, currentTrackPath: next.path, currentTime: 0, playing: true });
  },
  playPrevious: () => {
    const { currentTrack } = get();
    const tracks = useMusicStore.getState().tracks;
    const previous = getPreviousTrack(tracks, currentTrack);

    if (!previous) {
      set({ playing: false, currentTime: 0 });
      return;
    }

    set({ currentTrack: previous, currentTrackPath: previous.path, currentTime: 0, playing: true });
  },
}));

export const playerStoreHandler = createTauriStore(PLAYER_STORE_ID, usePlayerStore, {
  saveOnChange: true,
  saveOnExit: false,
  filterKeys: ['volume', 'currentTrackPath'],
  filterKeysStrategy: 'pick',
});
