import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const PlayInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        d='M12 3C7.032 3 3 7.032 3 12C3 16.968 7.032 21 12 21C16.968 21 21 16.968 21 12C21 7.032 16.968 3 12 3ZM9.75 14.403V9.597C9.75 8.886 10.542 8.454 11.136 8.841L14.871 11.244C14.9973 11.3256 15.1012 11.4375 15.1731 11.5696C15.245 11.7017 15.2827 11.8496 15.2827 12C15.2827 12.1504 15.245 12.2983 15.1731 12.4304C15.1012 12.5625 14.9973 12.6744 14.871 12.756L11.136 15.159C10.542 15.546 9.75 15.114 9.75 14.403Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Play = memo(PlayInner);
Play.displayName = 'Play';
