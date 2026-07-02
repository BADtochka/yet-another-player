import { type VolumeNormalizationLevel } from '@/types/Settings';
import { type Track } from '@/types/Track';

const LEVEL_OFFSET_DB: Record<VolumeNormalizationLevel, number> = {
  quiet: -16,
  normal: -10.67,
  loud: -5.33,
};

const MINIMAL_GAIN_DB = -2.67;

const dbToLinear = (db: number) => 10 ** (db / 20);

const clampVolume = (value: number) => Math.min(1, Math.max(0, value));

export const getPlaybackVolume = (
  userVolume: number,
  track: Track | null,
  enabled: boolean,
  level: VolumeNormalizationLevel,
) => {
  const adjustmentDb = enabled
    ? track?.replayGainDb != null
      ? track.replayGainDb + LEVEL_OFFSET_DB[level]
      : LEVEL_OFFSET_DB[level]
    : MINIMAL_GAIN_DB;

  return clampVolume(userVolume * dbToLinear(adjustmentDb));
};
