import { type Track } from './Track';

export type Artist = {
  name: string;
  avatarBase64: string;
  tracks: Track[];
};
