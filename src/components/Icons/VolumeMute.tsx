import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const VolumeMuteInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M3.74854 9.09522V15.0172H7.11979L12.1673 19.1617L12.1695 4.94922L7.12204 9.09447L3.74854 9.09522Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const VolumeMute = memo(VolumeMuteInner);
VolumeMute.displayName = 'VolumeMute';
