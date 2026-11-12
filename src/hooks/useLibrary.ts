import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'preact/hooks';
import { create } from 'zustand';
import { type Track } from '@/types/Track';
import { type TrackSort } from '@/stores/trackViewStore';
import { type CollectionSummary } from '@/utils/collections';

const useCatalogRevision = create<{ revision: number }>(() => ({ revision: 0 }));
export const invalidateCatalog = () => useCatalogRevision.setState(({ revision }) => ({ revision: revision + 1 }));
export const subscribeToCatalog = (listener: () => void) => useCatalogRevision.subscribe(listener);

export type CollectionKind = 'album' | 'artist' | 'playlist';
export type TrackQuery = { sort: TrackSort | 'path'; text: string; excludePlaylist?: number };
type Cursor = { id: number; revision: number };
type Page = { tracks: Track[]; nextCursor: Cursor | null; revision: number };
type LoadedPage = Page & { key: string };

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

export const useYoutubeTracks = (ids: string[]) =>
  useLibraryResource(
    JSON.stringify(ids),
    () => invoke<Record<string, Track>>('library_youtube_tracks', { ids }),
    ids.length > 0,
  );

const readPage = (query: TrackQuery, cursor: Cursor | null = null) =>
  invoke<Page>('library_tracks_page', { query, cursor });

export function useTrackPages(query: TrackQuery, enabled = true) {
  const revision = useCatalogRevision((state) => state.revision);
  const [attempt, setAttempt] = useState(0);
  const key = JSON.stringify([query.sort, query.text, query.excludePlaylist, revision, attempt]);
  const [loaded, setLoaded] = useState<LoadedPage>();
  const [failure, setFailure] = useState<{ key: string; message: string }>();
  const pending = useRef<LoadedPage>();
  const page = enabled && loaded?.key === key ? loaded : undefined;
  const error = enabled && failure?.key === key ? failure.message : undefined;

  useEffect(() => {
    if (!enabled) return;
    let active = true;
    void readPage(query).then(
      (result) => {
        if (!active) return;
        setFailure(undefined);
        setLoaded({ ...result, key });
      },
      (reason) => { if (active) setFailure({ key, message: String(reason) }); },
    );
    return () => {
      active = false;
      pending.current = undefined;
    };
  }, [key, enabled]);

  const loadMore = () => {
    if (!page?.nextCursor || pending.current === page) return;
    pending.current = page;
    setFailure(undefined);
    void readPage(query, page.nextCursor).then(
      (result) => {
        if (pending.current !== page) return;
        setLoaded((current) => current !== page ? current : {
          ...result,
          key,
          tracks: result.revision === page.revision ? [...page.tracks, ...result.tracks] : result.tracks,
        });
      },
      (reason) => { if (pending.current === page) setFailure({ key, message: String(reason) }); },
    ).finally(() => {
      if (pending.current === page) pending.current = undefined;
    });
  };

  return {
    tracks: page?.tracks ?? [],
    hasMore: Boolean(page?.nextCursor),
    loading: enabled && !page && !error,
    error,
    loadMore,
    retry: () => page ? loadMore() : setAttempt((count) => count + 1),
  };
}
