import { useEffect, useRef } from 'preact/hooks';
import { VolumeMax } from '@/components/Icons/VolumeMax';
import { VolumeMid } from '@/components/Icons/VolumeMid';
import { VolumeMin } from '@/components/Icons/VolumeMin';
import { VolumeMute } from '@/components/Icons/VolumeMute';
import { Slider } from '@/components/Slider';

export type VolumeBarProps = {
  value: number;
  className?: string;
  onChange: (value: number) => void;
};

const volumeIcon = (value: number) => {
  if (value === 0) {
    return VolumeMute;
  }

  if (value < 0.34) {
    return VolumeMin;
  }

  if (value < 0.67) {
    return VolumeMid;
  }

  return VolumeMax;
};

const VOLUME_WHEEL_STEP = 0.05;

const clampVolume = (value: number) => Math.min(1, Math.max(0, value));

export const VolumeBar = ({ value, className, onChange }: VolumeBarProps) => {
  const previousVolumeRef = useRef(0.5);
  const valueRef = useRef(value);
  const sliderRef = useRef<HTMLDivElement>(null);
  const Icon = volumeIcon(value);

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
      const next = clampVolume(Math.round((valueRef.current + direction * VOLUME_WHEEL_STEP) * 100) / 100);

      if (next > 0) {
        previousVolumeRef.current = next;
      }

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
      <button
        type='button'
        className='inline-flex size-6 shrink-0 cursor-pointer items-center justify-center text-white/80 transition-colors hover:text-white'
        aria-label={value === 0 ? 'Unmute' : 'Mute'}
        onPointerDown={onMuteToggle}
      >
        <Icon size={24} />
      </button>
      <div ref={sliderRef} className='w-24'>
        <Slider
          className='w-full'
          value={value}
          min={0}
          max={1}
          step={0.01}
          showThumb={false}
          onValueChange={onChange}
          onValueCommit={onChange}
        />
      </div>
    </div>
  );
};
