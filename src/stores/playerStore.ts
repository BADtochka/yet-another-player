import { saveNow } from '@tauri-store/zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { create } from 'zustand';
import { subscribeToCatalog, tracksByPaths, type TrackQuery } from '@/hooks/useLibrary';
import { useSettingsStore } from '@/stores/settingsStore';
import { type Track } from '@/types/Track';
import { type VolumeNormalizationLevel } from '@/types/Settings';
import { toast } from '@/utils/toast';

type RepeatMode = 'off' | 'all' | 'one';

export type AudioPreview = {
  id: number;
  title: string;
  artist: string;
  loading: boolean;
  playing: boolean;
  position: number;
  duration: number | null;
};

type AudioSnapshot = {
  revision: number;
  queueGeneration: number;
  queueIndex: number;
  queueLength: number;
  queuePaths?: string[];
  nextPath: string | null;
  shuffle: boolean;
  repeat: RepeatMode;
  path: string | null;
  playing: boolean;
  position: number;
  duration: number | null;
  volume: number;
  error: string | null;
  preview: AudioPreview | null;
};

type AudioAction =
  | { type: 'snapshot' | 'next' | 'previous' | 'toggleShuffle' | 'cycleRepeat' }
  | { type: 'append'; track: Pick<Track, 'path' | 'replayGainDb'> }
  | { type: 'jump'; index: number; generation: number }
  | { type: 'configure'; volume: number; normalization: VolumeNormalizationLevel | 'off' }
  | { type: 'queue'; tracks: Pick<Track, 'path' | 'replayGainDb'>[]; index: number; position: number; playing: boolean }
  | { type: 'playing'; playing: boolean }
  | { type: 'volume'; volume: number }
  | { type: 'preparePreview'; title: string; artist: string }
  | { type: 'closePreview'; id: number; keepPaused?: boolean }
  | { type: 'previewPlaying'; id: number; playing: boolean }
  | { type: 'previewSeek'; id: number; position: number }
  | { type: 'seek'; path: string; position: number };

type PlayerState = {
  queueGeneration: number;
  queueIndex: number;
  shuffle: boolean;
  repeat: RepeatMode;
  toggleShuffle: () => void;
  cycleRepeat: () => void;
  jumpToQueueItem: (index: number, generation: number) => void;
  volume: number;
  currentTrack: Track | null;
  currentTrackPath: string | null;
  nextTrackPath: string | null;
  playing: boolean;
  currentTime: number;
  duration: number;
  preview: AudioPreview | null;
  setVolume: (volume: number) => void;
  setPlaying: (playing: boolean) => void;
  commitSeek: (path: string, time: number) => void;
  playTrack: (track: Track, queue: Track[]) => void;
  playNext: () => void;
  playPrevious: () => void;
};

const PLAYER_STORE_ID = 'player';
let lastSnapshot: AudioSnapshot | undefined;
let lastSavedAt = 0;
let saveInFlight: Promise<void> | undefined;
let trackLookup = 0;

const refreshCurrentTrack = (path: string | null) => {
  const lookup = ++trackLookup;
  if (!path) return;
  void tracksByPaths([path]).then(([track]) => {
    if (lookup === trackLookup && usePlayerStore.getState().currentTrackPath === path) {
      const duration = lastSnapshot?.duration ?? track?.duration ?? 0;
      usePlayerStore.setState({ currentTrack: track ?? null, duration });
    }
  }).catch((error) => console.warn('Could not load track metadata', error));
};

const persistedState = () => {
  const { volume, currentTrackPath, currentTime } = usePlayerStore.getState();
  return { volume, currentTrackPath, currentTime };
};

const savePlayerState = () => {
  const previousSave = saveInFlight ?? Promise.resolve();
  const state = persistedState();
  const write = previousSave.catch(() => undefined).then(async () => {
    await invoke('plugin:zustand|patch', { id: PLAYER_STORE_ID, state });
    await saveNow(PLAYER_STORE_ID);
  });
  saveInFlight = write;
  return write;
};

const applySnapshot = (snapshot: AudioSnapshot) => {
  const previous = lastSnapshot;
  if (previous && snapshot.revision <= previous.revision) {
    return;
  }
  lastSnapshot = snapshot;
  const state = usePlayerStore.getState();
  const currentTrack = snapshot.path === state.currentTrackPath ? state.currentTrack : null;

  usePlayerStore.setState({
    queueGeneration: snapshot.queueGeneration,
    queueIndex: snapshot.queueIndex,
    shuffle: snapshot.shuffle,
    repeat: snapshot.repeat,
    currentTrack,
    currentTrackPath: snapshot.path,
    nextTrackPath: snapshot.nextPath,
    playing: snapshot.playing,
    currentTime: snapshot.position,
    duration: snapshot.duration ?? currentTrack?.duration ?? 0,
    volume: snapshot.volume,
    preview: snapshot.preview,
  });
  if (snapshot.error && snapshot.error !== previous?.error) {
    toast.error(snapshot.error);
  }

  const changedTrack = previous?.path !== snapshot.path;
  if (changedTrack) refreshCurrentTrack(snapshot.path);
  const paused = previous?.playing && !snapshot.playing;
  const checkpoint = snapshot.playing && Date.now() - lastSavedAt >= 5000;
  if (snapshot.path && (changedTrack || paused || checkpoint)) {
    lastSavedAt = Date.now();
    void savePlayerState().catch((error) => console.warn('Could not save playback position', error));
  }
};

const send = async (action: AudioAction) => {
  const snapshot = await invoke<AudioSnapshot>('audio_control', { action });
  applySnapshot(snapshot);
  if (action.type === 'seek' && snapshot.path === action.path && !snapshot.error) {
    void savePlayerState().catch((error) => console.warn('Could not save playback position', error));
  }
  return snapshot;
};

const dispatch = (action: AudioAction) => {
  void send(action).catch((error) => toast.error('Не удалось выполнить команду плеера', error));
};

const queueAction = (queue: Track[], index: number, position: number, playing: boolean): AudioAction => ({
  type: 'queue',
  tracks: queue.map(({ path, replayGainDb }) => ({ path, replayGainDb })),
  index,
  position,
  playing,
});

export const playDownloadedTrack = async (track: Track) => {
  const snapshot = await send(queueAction([track], 0, 0, true));
  if (snapshot.error) return false;
  return snapshot.path === track.path && snapshot.playing;
};

type QueueTarget = { type: 'id'; id: number } | { type: 'path'; path: string } | { type: 'first' | 'last' };
const libraryOrder: TrackQuery = { sort: 'path', text: '' };

const startTrackQuery = async (query: TrackQuery, target: QueueTarget, position: number, playing: boolean) => {
  const snapshot = await invoke<AudioSnapshot | null>('library_play_tracks', { query, target, position, playing });
  if (snapshot) applySnapshot(snapshot);
  return snapshot;
};

export const playTrackQuery = (query: TrackQuery, target: QueueTarget) =>
  void startTrackQuery(query, target, 0, true).catch((error) => toast.error('Не удалось открыть очередь', error));

export const savePlayerStateOnExit = async () => {
  const { preview } = await send({ type: 'snapshot' });
  if (preview) await send({ type: 'closePreview', id: preview.id, keepPaused: true });
  await savePlayerState();
};

export const preparePreview = async (title: string, artist: string) => {
  const { preview } = await send({ type: 'preparePreview', title, artist });
  if (!preview) throw new Error('Предпросмотр отменён.');
  return preview.id;
};

export const closePreview = (id: number) => dispatch({ type: 'closePreview', id });
export const setPreviewPlaying = (id: number, playing: boolean) => dispatch({ type: 'previewPlaying', id, playing });
export const seekPreview = (id: number, position: number) => dispatch({ type: 'previewSeek', id, position });

export const startPlayer = async () => {
  const saved = await invoke<{ volume?: number; currentTrackPath?: string; currentTime?: number }>(
    'plugin:zustand|load',
    { id: PLAYER_STORE_ID },
  );
  const snapshot = await invoke<AudioSnapshot>('audio_control', { action: { type: 'snapshot' } });
  await listen<AudioSnapshot>('audio:state', (event) => applySnapshot(event.payload));

  const { volumeNormEnabled, volumeNormLevel } = useSettingsStore.getState();
  const volume = snapshot.path ? snapshot.volume : saved.volume ?? 0.1;
  await send({ type: 'configure', volume, normalization: volumeNormEnabled ? volumeNormLevel : 'off' });
  if (!snapshot.path && saved.currentTrackPath) {
    const position = Number.isFinite(saved.currentTime) ? Math.max(0, saved.currentTime ?? 0) : 0;
    if (!await startTrackQuery(libraryOrder, { type: 'path', path: saved.currentTrackPath }, position, false)) {
      const path = await invoke<string | null>('library_resolve_path', { path: saved.currentTrackPath });
      if (path && path !== saved.currentTrackPath) {
        await startTrackQuery(libraryOrder, { type: 'path', path }, position, false);
      }
    }
  }

  useSettingsStore.subscribe((state, previous) => {
    if (state.volumeNormEnabled !== previous.volumeNormEnabled || state.volumeNormLevel !== previous.volumeNormLevel) {
      dispatch({
        type: 'configure',
        volume: usePlayerStore.getState().volume,
        normalization: state.volumeNormEnabled ? state.volumeNormLevel : 'off',
      });
    }
  });

  subscribeToCatalog(() => refreshCurrentTrack(usePlayerStore.getState().currentTrackPath));
};

export const addTrackToQueue = ({ path, replayGainDb }: Pick<Track, 'path' | 'replayGainDb'>) =>
  send({ type: 'append', track: { path, replayGainDb } });

export const inspectQueue = async (offset: number) => {
  const { queuePaths = [], queueGeneration, queueLength } = await invoke<AudioSnapshot>('audio_control', {
    action: { type: 'inspectQueue', offset },
  });
  const metadata = new Map((await tracksByPaths(queuePaths)).map((track) => [track.path, track]));
  const entries = queuePaths.map((path) => ({ path, track: metadata.get(path) }));
  return { entries, offset, generation: queueGeneration, length: queueLength };
};

export const usePlayerStore = create<PlayerState>((_set, get) => ({
  queueGeneration: 0,
  queueIndex: 0,
  nextTrackPath: null,
  shuffle: false,
  repeat: 'off',
  toggleShuffle: () => dispatch({ type: 'toggleShuffle' }),
  cycleRepeat: () => dispatch({ type: 'cycleRepeat' }),
  jumpToQueueItem: (index, generation) => dispatch({ type: 'jump', index, generation }),
  volume: 0.1,
  currentTrack: null,
  currentTrackPath: null,
  playing: false,
  currentTime: 0,
  duration: 0,
  preview: null,
  setVolume: (volume) => dispatch({ type: 'volume', volume }),
  setPlaying: (playing) => dispatch({ type: 'playing', playing }),
  commitSeek: (path, position) => dispatch({ type: 'seek', path, position }),
  playTrack: (track, queue) => {
    const index = queue.findIndex((item) => item.path === track.path);
    if (index !== -1) {
      dispatch(queueAction(queue, index, 0, true));
    }
  },
  playNext: () => {
    if (get().currentTrackPath) {
      dispatch({ type: 'next' });
    } else {
      playTrackQuery(libraryOrder, { type: 'first' });
    }
  },
  playPrevious: () => {
    if (get().currentTrackPath) {
      dispatch({ type: 'previous' });
    } else {
      playTrackQuery(libraryOrder, { type: 'last' });
    }
  },
}));
