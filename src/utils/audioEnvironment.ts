import { invoke } from '@tauri-apps/api/core';

export type AudioEnvironment = {
  platform: string;
  isWsl: boolean;
  pulseServer: string | null;
  pulseLatencyMs: string | null;
  webAudioRecommended: boolean;
  webAudioDisabledByEnv: boolean;
  notes: string[];
};

let audioEnvironmentPromise: Promise<AudioEnvironment> | null = null;

export const getAudioEnvironment = () => {
  if (!audioEnvironmentPromise) {
    audioEnvironmentPromise = invoke<AudioEnvironment>('get_audio_environment');
  }

  return audioEnvironmentPromise;
};
