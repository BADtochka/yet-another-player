import { type Track } from '@/types/Track';

const trackIndex = (tracks: Track[], current: Track | null) => {
  if (!current) {
    return -1;
  }

  return tracks.findIndex((track) => track.path === current.path);
};

export const getNextTrack = (tracks: Track[], current: Track | null): Track | null => {
  if (tracks.length === 0) {
    return null;
  }

  const index = trackIndex(tracks, current);
  const nextIndex = index === -1 ? 0 : (index + 1) % tracks.length;

  return tracks[nextIndex] ?? null;
};

export const getPreviousTrack = (tracks: Track[], current: Track | null): Track | null => {
  if (tracks.length === 0) {
    return null;
  }

  const index = trackIndex(tracks, current);
  const previousIndex = index === -1 ? tracks.length - 1 : (index - 1 + tracks.length) % tracks.length;

  return tracks[previousIndex] ?? null;
};
