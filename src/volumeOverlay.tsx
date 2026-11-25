import { render } from 'preact';
import { VolumeIndicator, useVolumeLevel } from '@/components/VolumeIndicator';
import '@fontsource/onest/400.css';
import '@fontsource/onest/700.css';
import '@/kit.css';
import '@/volumeIndicator.css';

function VolumeOverlay() {
  const level = useVolumeLevel();
  return level !== null ? <VolumeIndicator level={level} /> : null;
}

render(<VolumeOverlay />, document.getElementById('root')!);
