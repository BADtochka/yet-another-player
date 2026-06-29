import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const NextInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M15.366 11.28L4.5615 3.8565C3.7035 3.282 3 3.7035 3 4.794V19.2075C3 20.295 3.7035 20.7165 4.5615 20.145L15.366 12.7185C15.366 12.7185 15.7845 12.42 15.7845 12.0015C15.7845 11.5815 15.366 11.28 15.366 11.28ZM18 3H19.5C20.3295 3 21 3.072 21 3.9V20.1C21 20.928 20.3295 21 19.5 21H18C17.1705 21 16.5 20.928 16.5 20.1V3.9C16.5 3.072 17.1705 3 18 3Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Next = memo(NextInner);
Next.displayName = 'Next';
