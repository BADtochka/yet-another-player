import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'preact/hooks';
import { create } from 'zustand';
import { type Track } from '@/types/Track';
import { type TrackSort } from '@/stores/trackViewStore';
import { toast } from '@/utils/toast';
import { type CollectionSummary } from '@/utils/collections';

export const useMetadataLookup = create<{ running: boolean }>(() => ({ running: false }));
export const useCatalogRevision = create<{ revision: number }>(() => ({ revision: 0 }));
export const invalidateCatalog = () => useCatalogRevision.setState(({ revision }) => ({ revision: revision + 1 }));
export const listenForCatalogChanges = async () => {
  const listeners = await Promise.allSettled([
    listen('library:metadata', invalidateCatalog),
    listen<boolean>('library:metadata-running', ({ payload }) => useMetadataLookup.setState({ running: payload })),
    listen('library:metadata-error', () => toast.error('Не удалось закончить поиск метаданных в OMMR')),
  ]);
  return () => {
    for (const listener of listeners) {
      if (listener.status === 'fulfilled') listener.value();
    }
  };
};
export const subscribeToCatalog = (listener: () => void) => useCatalogRevision.subscribe(listener);

export type CollectionKind = 'album' | 'artist' | 'playlist';
export type TrackQuery = { sort: TrackSort | 'path'; text: string; excludePlaylist?: number };
type Cursor = { id: number; revision: number };
type Page = { tracks: Track[]; nextCursor: Cursor | null; revision: number };
type LoadedPage = Page & { queryKey: string; requestKey: string };

export const tracksByPaths = (paths: string[]) => invoke<Track[]>('library_tracks_by_paths', { paths });

function useLibraryResource<T>(key: string, load: () => Promise<T>, enabled: boolean) {
  const revision = useCatalogRevision((state) => state.revision);
  const requestKey = JSON.stringify([key, revision]);
  const [result, setResult] = useState<{ key: string; data?: T; error?: string }>();
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    void load().then(
      (data) => { if (active) setResult({ key, data }); },
      (error) => { if (active) setResult({ key, error: String(error) }); },
    );
    return () => { active = false; };
  }, [requestKey, enabled]);
  return enabled && result?.key === key ? result : undefined;
}

export const useLibraryTrack = (path: string | null) =>
  useLibraryResource(
    JSON.stringify(path),
    () => path ? tracksByPaths([path]).then(([track]) => track) : Promise.resolve(undefined),
    path !== null,
  )?.data;

export const useCollections = (kind: CollectionKind, text = '', enabled = true) =>
  useLibraryResource(
    JSON.stringify([kind, text]),
    () => invoke<CollectionSummary[]>('library_collections', { kind, text }),
    enabled,
  );

export const useCollectionTracks = (kind: CollectionKind, id?: string) =>
  useLibraryResource(
    JSON.stringify([kind, id]),
    () => invoke<Track[]>('library_collection_tracks', { kind, id }),
    id !== undefined,
  );

export const useExternalTracks = (ids: string[]) =>
  useLibraryResource(
    JSON.stringify(ids),
    () => invoke<Record<string, Track>>('library_external_tracks', { ids }),
    ids.length > 0,
  );

const readPage = (query: TrackQuery, cursor: Cursor | null = null) =>
  invoke<Page>('library_tracks_page', { query, cursor });

export function useTrackPages(query: TrackQuery, enabled = true) {
  const revision = useCatalogRevision((state) => state.revision);
  const [attempt, setAttempt] = useState(0);
  const queryKey = JSON.stringify([query.sort, query.text, query.excludePlaylist]);
  const requestKey = JSON.stringify([queryKey, revision, attempt]);
  const [loaded, setLoaded] = useState<LoadedPage>();
  const lastPage = useRef<LoadedPage>();
  const [failure, setFailure] = useState<{ key: string; message: string }>();
  const pending = useRef<LoadedPage>();
  const [loadingMore, setLoadingMore] = useState(false);
  const page = enabled && loaded?.queryKey === queryKey ? loaded : undefined;
  const fresh = page?.requestKey === requestKey;
  const error = enabled && failure?.key === requestKey ? failure.message : undefined;

  useEffect(() => {
    if (!enabled) return;
    setLoadingMore(false);
    let active = true;
    const previous = lastPage.current;
    const visibleCount = previous?.queryKey === queryKey ? previous.tracks.length : 0;
    const refresh = async () => {
      const result = await readPage(query);
      while (active && result.nextCursor && result.tracks.length < visibleCount) {
        const next = await readPage(query, result.nextCursor);
        if (next.revision !== result.revision) return next;
        result.tracks.push(...next.tracks);
        result.nextCursor = next.nextCursor;
      }
      return result;
    };
    void refresh().then(
      (result) => {
        if (!active) return;
        setFailure(undefined);
        const next = { ...result, queryKey, requestKey };
        lastPage.current = next;
        setLoaded(next);
      },
      (reason) => { if (active) setFailure({ key: requestKey, message: String(reason) }); },
    );
    return () => {
      active = false;
      pending.current = undefined;
    };
  }, [requestKey, enabled]);

  const loadMore = () => {
    if (!fresh || !page?.nextCursor || pending.current === page) return;
    pending.current = page;
    setLoadingMore(true);
    setFailure(undefined);
    void readPage(query, page.nextCursor).then(
      (result) => {
        if (pending.current !== page) return;
        const next = {
          ...result,
          queryKey,
          requestKey,
          tracks: result.revision === page.revision ? [...page.tracks, ...result.tracks] : result.tracks,
        };
        lastPage.current = next;
        setLoaded(next);
      },
      (reason) => { if (pending.current === page) setFailure({ key: requestKey, message: String(reason) }); },
    ).finally(() => {
      if (pending.current === page) {
        pending.current = undefined;
        setLoadingMore(false);
      }
    });
  };

  return {
    tracks: page?.tracks ?? [],
    hasMore: Boolean(page?.nextCursor),
    loading: enabled && !fresh && !error,
    loadingMore: enabled && loadingMore,
    error,
    loadMore,
    retry: () => fresh ? loadMore() : setAttempt((count) => count + 1),
  };
}
