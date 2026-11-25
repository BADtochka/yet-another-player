export type VolumeNormalizationLevel = 'quiet' | 'normal' | 'loud';

export type TransitionPreset = 'auto' | 'blend' | 'linear' | 'dip' | 'rise' | 'bassSwap' | 'lowPass' | 'highPass' | 'cut' | 'outro' | 'hook';

export type PlaylistTransition = {
  preset: TransitionPreset;
  seconds: number;
  trim: number;
  cue: number;
  intensity: number;
};

export type TransitionSettings = {
  enabled: boolean;
  mode: 'smooth' | 'advanced';
  seconds: number;
  maxSeconds: number;
  preset: TransitionPreset;
  intensity: number;
  trimSilence: boolean;
  alignBeats: boolean;
};

export type ResolverMode = 'off' | 'local' | 'remote';

export type MetadataSource = 'auto' | 'ommr' | 'itunes' | 'musicBrainz' | 'genius' | 'off';
export type LyricsSource = 'auto' | 'local' | import('@/utils/lyrica').LyricaProvider | 'genius';
export type ToolName = 'yt-dlp' | 'ffmpeg' | 'ommr' | 'lyrica' | 'genius';

export type Settings = {
  metadataSource: MetadataSource;
  showExternalAlbumTracks: boolean;
  lyricsSource: LyricsSource;
  showAudioInfo: boolean;
  separateVideoLyrics: boolean;
  suspendHiddenVideo: boolean;
  lyricaEnabled: boolean;
  resolverMode?: ResolverMode;
  resolverSources: string[];
  resolverUrl: string;
  musicFolders: string[];
  volumeNormEnabled: boolean;
  volumeNormLevel: VolumeNormalizationLevel;
  restartOnPrevious: boolean;
  transitions: TransitionSettings;
};
