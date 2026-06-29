import { type Artist } from './Artist';

export type Track = {
  name: string;
  artists: Artist['name'][];
  lyrics: string[];
  coverBase64: string;
  path: string;
  album: string | null;
  year: number | null;
  duration: number | null;
};
