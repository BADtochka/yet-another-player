import { create } from 'zustand';
import { persist } from 'zustand/middleware';

type SearchEntry = { query: string; trackId?: number; title?: string; artist?: string };

export const useSearchHistory = create(persist(() => ({ entries: [] as SearchEntry[] }), {
  name: 'yap-search-history',
}));

export function addSearchHistory(entry: SearchEntry) {
  useSearchHistory.setState(({ entries }) => ({
    entries: [entry, ...entries.filter((item) => entry.trackId !== undefined
      ? item.trackId !== entry.trackId
      : item.trackId !== undefined || item.query.toLowerCase() !== entry.query.toLowerCase())].slice(0, 6),
  }));
}
