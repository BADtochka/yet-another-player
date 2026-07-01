export type VolumeNormalizationLevel = 'quiet' | 'normal' | 'loud';

export type Settings = {
  musicFolders: string[];
  volumeNormEnabled: boolean;
  volumeNormLevel: VolumeNormalizationLevel;
};
