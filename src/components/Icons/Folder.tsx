import type { SVGProps } from 'preact/compat';
import { forwardRef, memo } from 'preact/compat';

interface IconProps extends SVGProps<SVGSVGElement> {
  /** Icon size (default: 24) */
  size?: number;
}

const FolderInner = forwardRef<SVGSVGElement, IconProps>(function Folder({ size = 24, ...props }: IconProps) {
  return (
    <svg width={size} height={size} viewBox='0 0 24 24' fill='none' {...props}>
      <path
        d='M20 4H11.41L10 2.59C9.62 2.21 9.12 2 8.59 2H4C2.9 2 2 2.9 2 4V18C2 19.1 2.9 20 4 20H20C21.1 20 22 19.1 22 18V6C22 4.9 21.1 4 20 4ZM20 18H4V6H20V18Z'
        fill='currentColor'
      />
    </svg>
  );
});

export const Folder = memo(FolderInner);
Folder.displayName = 'Folder';
