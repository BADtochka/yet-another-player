import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const RepeatInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M7 21L3 17.4L7 13.8L8.4 15.105L6.85 16.5H17V12.9H19V18.3H6.85L8.4 19.695L7 21ZM5 11.1V5.7H17.15L15.6 4.305L17 3L21 6.6L17 10.2L15.6 8.895L17.15 7.5H7V11.1H5Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Repeat = memo(RepeatInner);
Repeat.displayName = 'Repeat';
