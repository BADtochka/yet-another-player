import { type Track } from '@/types/Track';

export type CollectionSummary = {
  id: string;
  name: string;
  subtitle: string | null;
  coverPath: string | null;
  duration: number;
  trackCount: number;
  year: number | null;
};

export type Collection = Pick<CollectionSummary, 'id' | 'name' | 'subtitle'> & { tracks: Track[] };

export const catalogNames = new Intl.Collator('ru', { numeric: true, sensitivity: 'base' });

const plurals = new Intl.PluralRules('ru');
const trackNouns = { one: 'трек', few: 'трека', many: 'треков', other: 'треков', zero: 'треков', two: 'трека' };
export const formatTrackCount = (count: number) => `${count} ${trackNouns[plurals.select(count)]}`;
