import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const PauseInner = forwardRef<SVGSVGElement, IconProps>(function Pause({ size = 24, ...props }: IconProps, ref) {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M12 3C7.041 3 3 7.041 3 12C3 16.959 7.041 21 12 21C16.959 21 21 16.959 21 12C21 7.041 16.959 3 12 3ZM11.1 9.3V15.6H9.3V8.4H11.1V9.3ZM14.7 9.3V15.6H12.9V8.4H14.7V9.3Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Pause = memo(PauseInner);
Pause.displayName = 'Pause';
