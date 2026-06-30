import { type Track } from '@/types/Track';

export type Artist = {
  name: string;
  avatarBase64: string;
  tracks: Track[];
};
