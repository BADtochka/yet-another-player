import { type Artist } from '@/types/Artist';

export type Track = {
  id: number;
  name: string;
  artists: Artist['name'][];
  lyrics: string[];
  path: string;
  album: string | null;
  albumId: string;
  externalVideoId?: string;
  clipVideoId?: string;
  hasLocalVideo?: boolean;
  albumArtist: string | null;
  discNumber: number | null;
  trackNumber: number | null;
  genre: string | null;
  year: number | null;
  duration: number | null;
  replayGainDb: number | null;
};
