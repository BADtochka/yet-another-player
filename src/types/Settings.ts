export type VolumeNormalizationLevel = 'quiet' | 'normal' | 'loud';

export type ResolverMode = 'off' | 'local' | 'remote';

export type Settings = {
  lyricaEnabled: boolean;
  resolverMode?: ResolverMode;
  resolverSources: string[];
  resolverUrl: string;
  musicFolders: string[];
  volumeNormEnabled: boolean;
  volumeNormLevel: VolumeNormalizationLevel;
};
