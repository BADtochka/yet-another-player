import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

type WindowCloseProps = SVGProps<SVGSVGElement>;

const WindowCloseInner = forwardRef<SVGSVGElement, WindowCloseProps>((props, ref) => {
  return (
    <svg width='14' height='14' viewBox='0 0 14 14' fill='none' ref={ref} {...props}>
      <path
        d='M1.4 0L0 1.4L5.6 7L0 12.6L1.4 14L7 8.4L12.6 14L14 12.6L8.4 7L14 1.4L12.6 0L7 5.6L1.4 0Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const WindowClose = memo(WindowCloseInner);
WindowClose.displayName = 'WindowClose';
