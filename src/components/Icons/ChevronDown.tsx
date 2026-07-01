import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const ChevronDownInner = forwardRef<SVGSVGElement, IconProps>(function ChevronDown(
  { size = 24, ...props }: IconProps,
  ref,
) {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' {...props}>
      <path
        d='M16.939 7.93896L12 12.879L7.06103 7.93896L4.93903 10.061L12 17.121L19.061 10.061L16.939 7.93896Z'
        fill='black'
      />
    </svg>
  );
});

export const ChevronDown = memo(ChevronDownInner);
ChevronDown.displayName = 'ChevronDown';
