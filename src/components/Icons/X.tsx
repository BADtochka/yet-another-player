import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const XInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M9.88005 12.7101L4.93005 17.6601L6.34005 19.0701L9.17005 16.2401L12.0001 13.4101L14.1201 15.5401L17.6601 19.0701L19.0701 17.6601L14.1201 12.7101L13.4101 12.0001L19.0701 6.34005L17.6601 4.93005L12.0001 10.5901L6.34005 4.93005L4.93005 6.34005L10.5901 12.0001L9.88005 12.7101Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const X = memo(XInner);
X.displayName = 'X';
