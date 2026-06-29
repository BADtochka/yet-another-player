import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

type WindowMinimizeProps = SVGProps<SVGSVGElement>;

const WindowMinimizeInner = forwardRef<SVGSVGElement, WindowMinimizeProps>((props, ref) => {
  return (
    <svg width='18' height='2' viewBox='0 0 18 2' fill='none' ref={ref} {...props}>
      <path d='M0 0H18V2H0V0Z' fill='currentColor' />
    </svg>
  );
});

export const WindowMinimize = memo(WindowMinimizeInner);
WindowMinimize.displayName = 'WindowMinimize';
