import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const ShuffleInner = forwardRef<SVGSVGElement, IconProps>(({ size = 24, ...props }: IconProps, ref) => {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' ref={ref} {...props}>
      <path
        fillRule='evenodd'
        clipRule='evenodd'
        d='M12.3656 12.7308L15.2694 16.1346L17.6239 16.1345L16.4844 14.7982L17.7313 13.3364L21 17.1682L17.7313 21L16.4843 19.5382L17.6239 18.2019H14.5382L11.1183 14.193L12.3656 12.7308ZM17.7313 3L21 6.83181L17.7313 10.6636L16.4843 9.20183L17.6239 7.86544H15.2694L6.45131 18.2019H3V16.1346H5.72015L14.5382 5.79819L17.6239 5.79814L16.4843 4.46179L17.7313 3ZM6.45131 5.79819L9.87128 9.80705L8.62394 11.2692L5.72015 7.86544H3V5.79819H6.45131Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Shuffle = memo(ShuffleInner);
Shuffle.displayName = 'Shuffle';
