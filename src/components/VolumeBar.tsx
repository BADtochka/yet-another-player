import { Button } from '@/components/Button';
import { Slider } from '@/components/Slider';
import { useEffect, useRef } from 'preact/hooks';

export type VolumeBarProps = {
  value: number;
  className?: string;
  onChange: (value: number) => void;
};

const volumeIcon = (value: number) => {
  if (value === 0) {
    return 'VolumeMute';
  }

  if (value < 0.34) {
    return 'VolumeMin';
  }

  if (value < 0.67) {
    return 'VolumeMid';
  }

  return 'VolumeMax';
};

const VOLUME_STEP = 0.1;

const clampVolume = (value: number) => Math.min(1, Math.max(0, value));

export const VolumeBar = ({ value, className, onChange }: VolumeBarProps) => {
  const previousVolumeRef = useRef(0.5);
  const valueRef = useRef(value);
  const sliderRef = useRef<HTMLDivElement>(null);
  const icon = volumeIcon(value);

  valueRef.current = value;

  useEffect(() => {
    const slider = sliderRef.current;
    if (!slider) {
      return;
    }

    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      event.stopPropagation();

      const direction = event.deltaY < 0 ? 1 : -1;
      const next = clampVolume(Math.round((valueRef.current + direction * VOLUME_STEP) * 100) / 100);
      if (next > 0) previousVolumeRef.current = next;
      onChange(next);
    };

    slider.addEventListener('wheel', onWheel, { passive: false });

    return () => {
      slider.removeEventListener('wheel', onWheel);
    };
  }, [onChange]);

  const onMuteToggle = () => {
    if (value > 0) {
      previousVolumeRef.current = value;
      onChange(0);
      return;
    }

    onChange(previousVolumeRef.current > 0 ? previousVolumeRef.current : 0.5);
  };

  return (
    <div className={className ?? 'flex items-center gap-2'}>
      <Button variant='ghost' size='icon' className='player-tool' icon={icon}
        aria-label={value === 0 ? 'Включить звук' : 'Выключить звук'} onClick={onMuteToggle} />
      <div ref={sliderRef} className='volume-slider'>
        <Slider
          label='Громкость'
          className='w-full'
          value={value}
          min={0}
          max={1}
          step={VOLUME_STEP}
          showThumb={false}
          onValueChange={onChange}
          onValueCommit={onChange}
        />
      </div>
    </div>
  );
};
