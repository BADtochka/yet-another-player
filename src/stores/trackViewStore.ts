import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export type TrackSort = 'name' | 'artist' | 'duration' | 'added';
export type LibrarySort = TrackSort | 'releaseYear' | 'created';

type TrackViewState = {
  layout: 'list' | 'grid';
  sort: LibrarySort;
  setLayout: (layout: TrackViewState['layout']) => void;
  setSort: (sort: LibrarySort) => void;
};

export const useTrackViewStore = create<TrackViewState>()(
  persist(
    (set) => ({
      layout: 'list',
      sort: 'name',
      setLayout: (layout) => set({ layout }),
      setSort: (sort) => set({ sort }),
    }),
    { name: 'yap-track-view' },
  ),
);
