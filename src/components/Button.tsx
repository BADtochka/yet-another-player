import { cn } from 'badlib';
import type { ComponentChildren, JSX } from 'preact';
import { Icon, type IconName } from '@/components/Icons/Icon';

type ButtonProps = JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'primary' | 'danger' | 'secondary' | 'ghost';
  size?: 'default' | 'small' | 'icon';
  icon?: IconName;
  children?: ComponentChildren;
};

export function Button({ variant = 'secondary', size = 'default', icon, children, className, type = 'button', ...props }: ButtonProps) {
  return (
    <button type={type} className={cn('kit-button', className)} data-variant={variant} data-size={size} {...props}>
      {icon && <Icon name={icon} size={size === 'small' ? 18 : 24} />}
      {children}
    </button>
  );
}
