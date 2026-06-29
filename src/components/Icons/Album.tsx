import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const AlbumInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M6 2H18V4H6V2ZM19 6H5C3.9 6 3 6.9 3 8V20C3 21.1 3.9 22 5 22H19C20.1 22 21 21.1 21 20V8C21 6.9 20.1 6 19 6ZM16.5 17.25C16.5 18.21 15.72 19 14.75 19C13.78 19 13 18.21 13 17.25C13 16.29 13.78 15.5 14.75 15.5C15.02 15.5 15.27 15.57 15.5 15.68V10.93L10.5 10.1V16.26C10.5 17.22 9.72 18.01 8.75 18.01C7.78 18.01 7 17.22 7 16.26C7 15.3 7.78 14.51 8.75 14.51C9.02 14.51 9.27 14.58 9.5 14.69V9.51C9.5 9.36 9.56 9.22 9.68 9.13C9.8 9.04 9.94 9 10.09 9.02L16.09 10.02C16.33 10.06 16.51 10.27 16.51 10.51V17.26L16.5 17.25Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Album = memo(AlbumInner);
Album.displayName = 'Album';
