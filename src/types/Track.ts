import { type Artist } from '@/types/Artist';

export type Track = {
  name: string;
  artists: Artist['name'][];
  lyrics: string[];
  path: string;
  album: string | null;
  year: number | null;
  duration: number | null;
  replayGainDb: number | null;
};
