import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const PreviousInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M8.634 11.28L19.4385 3.8565C20.2965 3.282 21 3.7035 21 4.794V19.2075C21 20.295 20.2965 20.7165 19.4385 20.145L8.634 12.7185C8.634 12.7185 8.2155 12.42 8.2155 12.0015C8.2155 11.5815 8.634 11.28 8.634 11.28ZM6 3H4.5C3.6705 3 3 3.072 3 3.9V20.1C3 20.928 3.6705 21 4.5 21H6C6.8295 21 7.5 20.928 7.5 20.1V3.9C7.5 3.072 6.8295 3 6 3Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Previous = memo(PreviousInner);
Previous.displayName = 'Previous';
