import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const ChevronUpInner = forwardRef<SVGSVGElement, IconProps>(function ChevronUp(
  { size = 24, ...props }: IconProps,
  ref,
) {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' {...props}>
      <path
        d='M12 6.87903L4.93903 13.939L7.06103 16.061L12 11.121L16.939 16.061L19.061 13.939L12 6.87903Z'
        fill='black'
      />
    </svg>
  );
});

export const ChevronUp = memo(ChevronUpInner);
ChevronUp.displayName = 'ChevronUp';
