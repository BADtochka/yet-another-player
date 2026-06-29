import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

type WindowMaximizeProps = SVGProps<SVGSVGElement>;

const WindowMaximizeInner = forwardRef<SVGSVGElement, WindowMaximizeProps>((props, ref) => {
  return (
    <svg width='18' height='18' viewBox='0 0 18 18' fill='none' ref={ref} {...props}>
      <path
        d='M15 2H3C2.45 2 2 2.45 2 3V15C2 15.55 2.45 16 3 16H15C15.55 16 16 15.55 16 15V3C16 2.45 15.55 2 15 2ZM14 14H4V4H14V14Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const WindowMaximize = memo(WindowMaximizeInner);
WindowMaximize.displayName = 'WindowMaximize';
