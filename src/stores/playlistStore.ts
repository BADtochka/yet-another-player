import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import { invalidateCatalog } from '@/hooks/useLibrary';
import type { PlaylistTransition } from '@/types/Settings';

export type Playlist = {
  id: number;
  name: string;
  trackIds: number[];
  mixEnabled: boolean;
  transitions: { outgoingId: number; incomingId: number; settings: PlaylistTransition }[];
};

type Change =
  | { type: 'list' }
  | { type: 'create'; name: string; mixEnabled: boolean }
  | { type: 'rename'; id: number; name: string }
  | { type: 'delete'; id: number }
  | { type: 'add' | 'remove'; id: number; trackId: number }
  | { type: 'move'; id: number; trackId: number; later: boolean }
  | { type: 'setMix'; id: number; enabled: boolean }
  | { type: 'setTransition'; id: number; outgoingId: number; incomingId: number; settings: PlaylistTransition | null };

export const usePlaylistStore = create<{ playlists: Playlist[] }>(() => ({ playlists: [] }));

let pending: Promise<unknown> = Promise.resolve();
export const changePlaylists = (change: Change) => {
  const operation = pending.then(async () => {
    const playlists = await invoke<Playlist[]>('library_playlists', { change });
    usePlaylistStore.setState({ playlists });
    if (change.type !== 'list' && change.type !== 'setMix' && change.type !== 'setTransition') invalidateCatalog();
    return playlists;
  });
  pending = operation.catch(() => undefined);
  return operation;
};
