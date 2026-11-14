import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';
import { useCatalogRevision } from '@/hooks/useLibrary';

type AudioInfo = {
  format: string;
  codec: string | null;
  bitrateKbps: number | null;
  sampleRateHz: number | null;
  bitDepth: number | null;
};

const sampleRate = new Intl.NumberFormat('ru', { maximumFractionDigits: 3 });

export function useTrackAudioInfo(path: string | undefined) {
  const revision = useCatalogRevision((state) => state.revision);
  const [result, setResult] = useState<{ path: string; info: AudioInfo }>();

  useEffect(() => {
    if (!path) return;
    let active = true;
    void invoke<AudioInfo>('track_audio_info', { path }).then((info) => {
      if (active) setResult({ path, info });
    }).catch(() => undefined);
    return () => { active = false; };
  }, [path, revision]);

  if (!result || result.path !== path) return undefined;
  const { format, codec, sampleRateHz, bitDepth, bitrateKbps } = result.info;
  return [
    format,
    codec,
    sampleRateHz && `${sampleRate.format(sampleRateHz / 1000)} кГц`,
    bitDepth && `${bitDepth} бит`,
    bitrateKbps && `${bitrateKbps} кбит/с`,
  ].filter(Boolean).join(' · ');
}
