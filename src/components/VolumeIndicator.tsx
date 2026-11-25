import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useEffect, useState } from 'preact/hooks';
import { Icon } from '@/components/Icons/Icon';

export function useVolumeLevel() {
  const [level, setLevel] = useState<number | null>(null);
  useEffect(() => {
    let active = true;
    const listener = getCurrentWebviewWindow().listen<number>('volume-gesture-level', ({ payload }) => {
      if (active) setLevel(payload);
    });
    void listener.then(() => {
      if (active) return invoke('volume_overlay_ready');
    }).catch((error) => console.warn('Could not open volume indicator', error));
    return () => {
      active = false;
      void listener.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);
  return level;
}

export function VolumeIndicator({ level }: { level: number }) {
  return <div className='volume-gesture-indicator' role='status' aria-label={`Громкость YAP ${level}%`}>
    <span className='volume-gesture-icon'><Icon name={level === 0 ? 'VolumeMute' : level < 60 ? 'VolumeMid' : 'VolumeMax'} /></span>
    <div className='volume-gesture-meter'>
      <span>Громкость YAP</span>
      <progress aria-label='Громкость' value={level} max={100} />
    </div>
    <strong>{level}%</strong>
  </div>;
}
