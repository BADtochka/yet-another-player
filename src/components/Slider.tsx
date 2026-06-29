import { type ComponentChildren } from 'preact';
import { useRef } from 'preact/hooks';

export type SliderProps = {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  showThumb?: boolean;
  disabled?: boolean;
  className?: string;
  onDragStart?: () => void;
  onValueChange?: (value: number) => void;
  onValueCommit?: (value: number) => void;
};

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

const snapToStep = (value: number, min: number, max: number, step: number) => {
  if (step <= 0) {
    return clamp(value, min, max);
  }

  const stepped = min + Math.round((value - min) / step) * step;
  return clamp(stepped, min, max);
};

const mergeClassName = (...parts: Array<string | false | undefined>) => parts.filter(Boolean).join(' ');

export const Slider = ({
  value,
  min = 0,
  max = 1,
  step = 0.1,
  showThumb = true,
  disabled = false,
  className,
  onDragStart,
  onValueChange,
  onValueCommit,
}: SliderProps) => {
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);

  const range = Math.max(max - min, 0);
  const normalizedValue = clamp(value, min, max);
  const percent = range === 0 ? 0 : ((normalizedValue - min) / range) * 100;

  const valueFromClientX = (clientX: number) => {
    const track = trackRef.current;
    if (!track) {
      return normalizedValue;
    }

    const rect = track.getBoundingClientRect();
    if (rect.width === 0) {
      return normalizedValue;
    }

    const ratio = clamp((clientX - rect.left) / rect.width, 0, 1);
    return snapToStep(min + ratio * range, min, max, step);
  };

  const onPointerDown = (event: PointerEvent) => {
    if (disabled) {
      return;
    }

    draggingRef.current = true;
    trackRef.current?.setPointerCapture(event.pointerId);
    onDragStart?.();

    const nextValue = valueFromClientX(event.clientX);
    onValueChange?.(nextValue);
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!draggingRef.current || disabled) {
      return;
    }

    onValueChange?.(valueFromClientX(event.clientX));
  };

  const onPointerUp = (event: PointerEvent) => {
    if (!draggingRef.current) {
      return;
    }

    draggingRef.current = false;
    trackRef.current?.releasePointerCapture(event.pointerId);
    onValueCommit?.(valueFromClientX(event.clientX));
  };

  return (
    <div
      ref={trackRef}
      role='slider'
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={normalizedValue}
      aria-disabled={disabled}
      className={mergeClassName(
        'relative flex h-11 w-full touch-none items-center select-none group',
        disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer',
        className,
      )}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <div className='pointer-events-none absolute inset-x-0 h-1.5 rounded-[10px] bg-[#182853]' />
      <div
        className='pointer-events-none absolute left-0 h-1.5 rounded-[10px] bg-[#d9d9d9]'
        style={{ width: `${percent}%` }}
      />
      {showThumb ? (
        <div
          className='pointer-events-none absolute size-3 -translate-x-1/2 rounded-full outline-[3px] outline-transparent group-hover:outline-[#7491e280] transition-colors bg-[#d9d9d9]'
          style={{ left: `${percent}%` }}
        />
      ) : null}
    </div>
  );
};

export type SliderFrameProps = {
  children: ComponentChildren;
  className?: string;
};

export const SliderFrame = ({ children, className }: SliderFrameProps) => (
  <div className={mergeClassName('flex w-full max-w-[468px] items-center gap-2', className)}>{children}</div>
);
