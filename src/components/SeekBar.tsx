import { Slider, type SliderProps } from './Slider';

export type SeekBarProps = Omit<SliderProps, 'showThumb' | 'onDragStart' | 'onValueChange' | 'onValueCommit'> & {
  onSeekStart?: () => void;
  onSeek?: (value: number) => void;
  onSeekEnd?: (value: number) => void;
};

export const SeekBar = ({
  onSeekStart,
  onSeek,
  onSeekEnd,
  step = 0.1,
  ...sliderProps
}: SeekBarProps) => (
  <Slider
    {...sliderProps}
    step={step}
    showThumb
    onDragStart={onSeekStart}
    onValueChange={onSeek}
    onValueCommit={onSeekEnd}
  />
);
