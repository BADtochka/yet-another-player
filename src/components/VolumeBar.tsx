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
const VOLUME_PER_PIXEL = 0.0008;
const VOLUME_PER_LINE = 0.025;
const WHEEL_GESTURE_GAP_MS = 180;

const clampVolume = (value: number) => Math.min(1, Math.max(0, value));

export const VolumeBar = ({ value, className, onChange }: VolumeBarProps) => {
  const previousVolumeRef = useRef(0.5);
  const valueRef = useRef(value);
  const sliderRef = useRef<HTMLDivElement>(null);
  const wheelRef = useRef<{ target: number; sent: number; lastEventAt: number }>();
  const wheelFrameRef = useRef<number>();
  const icon = volumeIcon(value);

  valueRef.current = value;

  const resetWheel = () => {
    if (wheelFrameRef.current !== undefined) cancelAnimationFrame(wheelFrameRef.current);
    wheelFrameRef.current = undefined;
    wheelRef.current = undefined;
  };

  useEffect(() => {
    const slider = sliderRef.current;
    if (!slider) {
      return;
    }

    const onWheel = (event: WheelEvent) => {
      const mode = event.deltaMode;
      const delta = event.deltaY;
      if (event.ctrlKey || !Number.isFinite(delta) || delta === 0 || Math.abs(event.deltaX) > Math.abs(delta)) return;
      event.preventDefault();
      event.stopPropagation();

      const scale = mode === WheelEvent.DOM_DELTA_LINE ? VOLUME_PER_LINE
        : mode === WheelEvent.DOM_DELTA_PAGE ? VOLUME_STEP : VOLUME_PER_PIXEL;
      const change = -Math.sign(delta) * Math.min(VOLUME_STEP, Math.abs(delta) * scale);
      const now = performance.now();
      const previous = wheelRef.current;
      // Native snapshots may arrive after the next event in the same gesture.
      const active = previous && now - previous.lastEventAt < WHEEL_GESTURE_GAP_MS ? previous : undefined;
      wheelRef.current = {
        target: clampVolume((active?.target ?? valueRef.current) + change),
        sent: active?.sent ?? valueRef.current,
        lastEventAt: now,
      };

      if (wheelFrameRef.current !== undefined) return;
      wheelFrameRef.current = requestAnimationFrame(() => {
        wheelFrameRef.current = undefined;
        const gesture = wheelRef.current;
        if (!gesture || gesture.target === gesture.sent) return;
        gesture.sent = gesture.target;
        if (gesture.target > 0) previousVolumeRef.current = gesture.target;
        onChange(gesture.target);
      });
    };

    slider.addEventListener('wheel', onWheel, { passive: false });

    return () => {
      slider.removeEventListener('wheel', onWheel);
      resetWheel();
    };
  }, [onChange]);

  const onMuteToggle = () => {
    resetWheel();
    if (value > 0) {
      previousVolumeRef.current = value;
      onChange(0);
      return;
    }

    onChange(previousVolumeRef.current > 0 ? previousVolumeRef.current : 0.5);
  };

  const changeDirectly = (next: number) => {
    resetWheel();
    onChange(next);
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
          onValueChange={changeDirectly}
          onValueCommit={changeDirectly}
        />
      </div>
    </div>
  );
};
