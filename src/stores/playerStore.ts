import { saveNow } from '@tauri-store/zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { create } from 'zustand';
import { subscribeToCatalog, tracksByPaths, type TrackQuery } from '@/hooks/useLibrary';
import { useSettingsStore } from '@/stores/settingsStore';
import { type Track } from '@/types/Track';
import { type TransitionPreset, type TransitionSettings, type VolumeNormalizationLevel } from '@/types/Settings';
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
  clipId: number | null;
  timeline: number;
  shuffle: boolean;
  repeat: RepeatMode;
  path: string | null;
  playing: boolean;
  position: number;
  duration: number | null;
  volume: number;
  error: string | null;
  preview: AudioPreview | null;
  transition?: TransitionPreview;
};

export type TransitionWaveform = {
  bpm: number | null;
  waveform: number[];
  windowSeconds: number;
  silence: number;
  duration: number | null;
  fadeSeconds: number | null;
  hook: number | null;
};

type TransitionPreview = {
  preset: TransitionPreset;
  seconds: number;
  fromPath: string;
  toPath: string;
  trim: number;
  cue: number;
  status: 'waiting' | 'analyzing' | 'ready' | 'unavailable';
  tail: TransitionWaveform | null;
  head: TransitionWaveform | null;
};

type NativeTransitionSettings = Omit<TransitionSettings, 'enabled' | 'mode'> & { mode: TransitionSettings['mode'] | 'off' };

type AudioAction =
  | { type: 'snapshot' | 'next' | 'previous' | 'toggleShuffle' | 'cycleRepeat' }
  | { type: 'append'; track: Pick<Track, 'path' | 'replayGainDb'> }
  | { type: 'jump'; index: number; generation: number }
  | { type: 'configure'; volume: number; normalization: VolumeNormalizationLevel | 'off'; restartOnPrevious: boolean }
  | { type: 'configureTransitions'; settings: NativeTransitionSettings }
  | { type: 'queue'; tracks: Pick<Track, 'path' | 'replayGainDb'>[]; index: number; position: number; playing: boolean }
  | { type: 'playing'; playing: boolean }
  | { type: 'volume'; volume: number }
  | { type: 'preparePreview'; title: string; artist: string }
  | { type: 'closePreview'; id: number; keepPaused?: boolean }
  | { type: 'previewPlaying'; id: number; playing: boolean }
  | { type: 'previewSeek'; id: number; position: number }
  | { type: 'stopClip'; id: number }
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
  nextTrack: Track | null;
  clipId: number | null;
  timeline: number;
  playing: boolean;
  currentTime: number;
  duration: number;
  preview: AudioPreview | null;
  transition: TransitionPreview | null;
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
let nextTrackLookup = 0;
let prefetchedPath: string | null = null;

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

const refreshNextTrack = (path: string | null) => {
  const lookup = ++nextTrackLookup;
  if (!path) return;
  void tracksByPaths([path]).then(([track]) => {
    if (lookup === nextTrackLookup && usePlayerStore.getState().nextTrackPath === path) {
      usePlayerStore.setState({ nextTrack: track ?? null });
    }
  }).catch((error) => console.warn('Could not load next track metadata', error));
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
  const currentTrack = snapshot.path === state.currentTrackPath
    ? state.currentTrack
    : snapshot.path === state.nextTrackPath ? state.nextTrack : null;
  const nextMetadataPath = snapshot.transition || useSettingsStore.getState().transitions.enabled ? snapshot.nextPath : null;
  const nextTrackChanged = nextMetadataPath !== prefetchedPath;

  usePlayerStore.setState({
    queueGeneration: snapshot.queueGeneration,
    queueIndex: snapshot.queueIndex,
    shuffle: snapshot.shuffle,
    repeat: snapshot.repeat,
    currentTrack,
    currentTrackPath: snapshot.path,
    nextTrackPath: snapshot.nextPath,
    nextTrack: nextTrackChanged ? null : state.nextTrack,
    clipId: snapshot.clipId,
    timeline: snapshot.timeline,
    playing: snapshot.playing,
    currentTime: snapshot.position,
    duration: snapshot.duration ?? currentTrack?.duration ?? 0,
    volume: snapshot.volume,
    preview: snapshot.preview,
    transition: snapshot.transition ?? null,
  });
  if (snapshot.error && snapshot.error !== previous?.error) {
    toast.error(snapshot.error);
  }

  const changedTrack = previous?.path !== snapshot.path;
  if (changedTrack) refreshCurrentTrack(snapshot.path);
  if (nextTrackChanged) {
    prefetchedPath = nextMetadataPath;
    refreshNextTrack(nextMetadataPath);
  }
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

export const playDownloadedTrack = async (track: Track, playing = true) => {
  const snapshot = await send(queueAction([track], 0, 0, playing));
  if (snapshot.error) return false;
  return snapshot.path === track.path && snapshot.playing === playing;
};

export const refreshPlayerSnapshot = () => send({ type: 'snapshot' });

export const startTrackClip = async (id: number, path: string, videoId: string | undefined) => {
  const snapshot = await invoke<AudioSnapshot>('start_track_clip', { id, path, videoId, autoplay: false });
  applySnapshot(snapshot);
  return snapshot.clipId === id;
};

export const stopTrackClip = (id: number) => dispatch({ type: 'stopClip', id });

type QueueTarget = { type: 'id'; id: number } | { type: 'path'; path: string } | { type: 'first' | 'last' };
const libraryOrder: TrackQuery = { sort: 'path', text: '' };

const startTrackQuery = async (query: TrackQuery, target: QueueTarget, position: number, playing: boolean) => {
  const snapshot = await invoke<AudioSnapshot | null>('library_play_tracks', { query, target, position, playing });
  if (snapshot) applySnapshot(snapshot);
  return snapshot;
};

export const playTrackQuery = (query: TrackQuery, target: QueueTarget) =>
  void startTrackQuery(query, target, 0, true).catch((error) => toast.error('Не удалось открыть очередь', error));

export const playSearchTrack = async (query: TrackQuery, track: Track, searchVideoIds: string[], keepPosition: boolean) => {
  const current = usePlayerStore.getState();
  const resume = keepPosition && current.currentTrackPath === track.path;
  const snapshot = await invoke<AudioSnapshot | null>('library_play_tracks', {
    query, target: { type: 'id', id: track.id }, searchVideoIds,
    position: resume ? current.currentTime : 0, playing: resume ? current.playing : true,
  });
  if (!snapshot) throw new Error('Трек недоступен в медиатеке.');
  applySnapshot(snapshot);
  return !snapshot.error && usePlayerStore.getState().currentTrackPath === track.path;
};

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

export const playPlaylist = (id: number, trackId: number) =>
  void invoke<AudioSnapshot>('library_play_playlist', { id, trackId }).then(applySnapshot)
    .catch((error) => toast.error('Не удалось воспроизвести плейлист', error));

export const startPlaylistTransitionPreview = async (
  previewId: number, id: number, outgoingId: number, incomingId: number, settings: import('@/types/Settings').PlaylistTransition,
) => {
  const snapshot = await invoke<AudioSnapshot>('library_preview_playlist_transition', {
    previewId, id, outgoingId, incomingId, settings,
  });
  applySnapshot(snapshot);
  if (snapshot.error) throw new Error(snapshot.error);
};

const transitionAction = ({ enabled, mode, ...settings }: TransitionSettings): AudioAction =>
  ({ type: 'configureTransitions', settings: { ...settings, mode: enabled ? mode : 'off' } });

export const startPlayer = async () => {
  const saved = await invoke<{ volume?: number; currentTrackPath?: string; currentTime?: number }>(
    'plugin:zustand|load',
    { id: PLAYER_STORE_ID },
  );
  const snapshot = await invoke<AudioSnapshot>('audio_control', { action: { type: 'snapshot' } });
  await listen<AudioSnapshot>('audio:state', (event) => applySnapshot(event.payload));
  await listen<string>('library:play-count', ({ payload }) => {
    if (payload === usePlayerStore.getState().currentTrackPath) refreshCurrentTrack(payload);
  });

  const { volumeNormEnabled, volumeNormLevel, restartOnPrevious, transitions } = useSettingsStore.getState();
  const volume = snapshot.path ? snapshot.volume : saved.volume ?? 0.1;
  await send({ type: 'configure', volume, normalization: volumeNormEnabled ? volumeNormLevel : 'off', restartOnPrevious });
  if (transitions.enabled) await send(transitionAction(transitions));
  if (!snapshot.path && saved.currentTrackPath) {
    const position = Number.isFinite(saved.currentTime) ? Math.max(0, saved.currentTime ?? 0) : 0;
    if (!await startTrackQuery(libraryOrder, { type: 'path', path: saved.currentTrackPath }, position, false)) {
      const path = await invoke<string | null>('library_resolve_path', { path: saved.currentTrackPath });
      if (path && path !== saved.currentTrackPath) {
        await startTrackQuery(libraryOrder, { type: 'path', path }, position, false);
      }
    }
  }

  let transitionUpdate = Promise.resolve();
  useSettingsStore.subscribe((state, previous) => {
    if (state.transitions !== previous.transitions && (state.transitions.enabled || previous.transitions.enabled)) {
      const action = transitionAction(state.transitions);
      transitionUpdate = transitionUpdate.then(async () => {
        await send(action);
      }).catch((error) => toast.error('Не удалось изменить режим переходов', error));
    }
    if (state.volumeNormEnabled !== previous.volumeNormEnabled || state.volumeNormLevel !== previous.volumeNormLevel
      || state.restartOnPrevious !== previous.restartOnPrevious) {
      dispatch({
        type: 'configure',
        volume: usePlayerStore.getState().volume,
        normalization: state.volumeNormEnabled ? state.volumeNormLevel : 'off',
        restartOnPrevious: state.restartOnPrevious,
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
  nextTrack: null,
  clipId: null,
  timeline: 0,
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
  transition: null,
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
