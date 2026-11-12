import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import { invalidateCatalog } from '@/hooks/useLibrary';

export type Playlist = { id: number; name: string; trackIds: number[] };

type Change =
  | { type: 'list' }
  | { type: 'create'; name: string }
  | { type: 'rename'; id: number; name: string }
  | { type: 'delete'; id: number }
  | { type: 'add' | 'remove'; id: number; trackId: number }
  | { type: 'move'; id: number; trackId: number; later: boolean };

export const usePlaylistStore = create<{ playlists: Playlist[] }>(() => ({ playlists: [] }));

let pending: Promise<unknown> = Promise.resolve();
export const changePlaylists = (change: Change) => {
  const operation = pending.then(async () => {
    const playlists = await invoke<Playlist[]>('library_playlists', { change });
    usePlaylistStore.setState({ playlists });
    if (change.type !== 'list') invalidateCatalog();
    return playlists;
  });
  pending = operation.catch(() => undefined);
  return operation;
};
