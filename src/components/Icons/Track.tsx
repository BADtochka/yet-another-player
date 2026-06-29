import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const TrackInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M21 3H9C8.45 3 8 3.45 8 4V13.56C7.41 13.22 6.73 13 6 13C3.79 13 2 14.79 2 17C2 19.21 3.79 21 6 21C8.21 21 10 19.21 10 17V5H20V13.56C19.41 13.22 18.73 13 18 13C15.79 13 14 14.79 14 17C14 19.21 15.79 21 18 21C20.21 21 22 19.21 22 17V4C22 3.45 21.55 3 21 3Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Track = memo(TrackInner);
Track.displayName = 'Track';
