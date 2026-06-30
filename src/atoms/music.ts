import { atom } from 'jotai';
import { type Track } from '@/types/Track';
import { getNextTrack, getPreviousTrack } from '@/utils/playQueue';

export const tracksAtom = atom<Track[]>([]);
export const volumeAtom = atom<number>(0.1);
export const currentTrackAtom = atom<Track | null>(null);
export const playingAtom = atom(false);
export const seekingAtom = atom(false);
export const currentTimeAtom = atom<number>(0);
export const durationAtom = atom<number>(0);

export const playNextAtom = atom(null, (get, set) => {
  const next = getNextTrack(get(tracksAtom), get(currentTrackAtom));

  if (!next) {
    set(playingAtom, false);
    set(currentTimeAtom, 0);
    return;
  }

  set(currentTrackAtom, next);
  set(currentTimeAtom, 0);
  set(playingAtom, true);
});

export const playPreviousAtom = atom(null, (get, set) => {
  const previous = getPreviousTrack(get(tracksAtom), get(currentTrackAtom));

  if (!previous) {
    set(playingAtom, false);
    set(currentTimeAtom, 0);
    return;
  }

  set(currentTrackAtom, previous);
  set(currentTimeAtom, 0);
  set(playingAtom, true);
});
