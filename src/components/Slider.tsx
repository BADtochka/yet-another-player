import { type ComponentChildren } from 'preact';
import { useRef } from 'preact/hooks';

export type SliderProps = {
  label: string;
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
  label,
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
    if (disabled || !event.isPrimary || event.button !== 0) {
      return;
    }

    draggingRef.current = true;
    trackRef.current?.focus({ preventScroll: true });
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
    if (trackRef.current?.hasPointerCapture(event.pointerId)) {
      trackRef.current.releasePointerCapture(event.pointerId);
    }
    onValueCommit?.(event.type === 'pointercancel' ? normalizedValue : valueFromClientX(event.clientX));
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (disabled || draggingRef.current) {
      return;
    }

    const keyboardStep = Math.max(step, range / 100);
    const targets: Record<string, number> = {
      ArrowLeft: normalizedValue - keyboardStep,
      ArrowDown: normalizedValue - keyboardStep,
      ArrowRight: normalizedValue + keyboardStep,
      ArrowUp: normalizedValue + keyboardStep,
      Home: min,
      End: max,
    };
    const target = targets[event.key];
    if (typeof target !== 'number') {
      return;
    }

    event.preventDefault();
    const next = snapToStep(target, min, max, step);
    onValueChange?.(next);
    onValueCommit?.(next);
  };

  return (
    <div
      ref={trackRef}
      role='slider'
      aria-label={label}
      tabIndex={disabled ? -1 : 0}
      onKeyDown={onKeyDown}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={normalizedValue}
      aria-disabled={disabled}
      className={mergeClassName('kit-slider', className)}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <div className='kit-slider-track' />
      <div
        className='kit-slider-fill'
        style={{ width: `${percent}%` }}
      />
      <div className='kit-slider-thumb' data-visibility={showThumb ? 'always' : 'interaction'}
        style={{ left: `${percent}%` }} />
    </div>
  );
};

export type SliderFrameProps = {
  children: ComponentChildren;
  className?: string;
};

export const SliderFrame = ({ children, className }: SliderFrameProps) => (
  <div className={mergeClassName('kit-slider-frame', className)}>{children}</div>
);
