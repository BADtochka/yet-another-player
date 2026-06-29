import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const PlaylistInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M2.8 19.98L12.8 21.98C12.87 21.99 12.93 22 13 22C13.23 22 13.45 21.92 13.63 21.77C13.86 21.58 14 21.3 14 21V3C14 2.7 13.87 2.42 13.63 2.23C13.4 2.04 13.1 1.97 12.8 2.02L2.8 4.02C2.33 4.11 2 4.52 2 5V19C2 19.48 2.34 19.89 2.8 19.98ZM16 2H18V22H16V2ZM20 2H22V22H20V2Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Playlist = memo(PlaylistInner);
Playlist.displayName = 'Playlist';
