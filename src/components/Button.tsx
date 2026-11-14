import { cn } from 'badlib';
import type { ComponentChildren, JSX } from 'preact';
import { Spinner } from '@/components/LoadingIndicator';
import { Icon, type IconName } from '@/components/Icons/Icon';

type ButtonProps = JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'primary' | 'danger' | 'secondary' | 'ghost';
  size?: 'default' | 'small' | 'icon';
  icon?: IconName;
  children?: ComponentChildren;
  pending?: boolean;
};

export function Button({ variant = 'secondary', size = 'default', icon, children, className, type = 'button', pending, disabled, ...props }: ButtonProps) {
  return (
    <button type={type} className={cn('kit-button', className)} data-variant={variant} data-size={size} {...props}
      disabled={disabled || pending} aria-busy={pending || props['aria-busy']}>
      {pending ? <Spinner size={size === 'small' ? 18 : icon || size === 'icon' ? 24 : undefined} />
        : icon && <Icon name={icon} size={size === 'small' ? 18 : 24} />}
      {children}
    </button>
  );
}
