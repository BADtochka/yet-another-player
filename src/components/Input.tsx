import { cn } from 'badlib';
import type { JSX } from 'preact';
import { forwardRef } from 'preact/compat';
import { Icon, type IconName } from '@/components/Icons/Icon';

type InputProps = Omit<JSX.InputHTMLAttributes<HTMLInputElement>, 'size'> & {
  icon?: IconName;
  submit?: boolean;
  onClear?: () => void;
};

export const Input = forwardRef<HTMLInputElement, InputProps>(({
  className, icon, submit, onClear, ...props
}, ref) => (
  <div className={cn('kit-input', className)} data-invalid={props['aria-invalid'] === true || props['aria-invalid'] === 'true'}>
    <input ref={ref} {...props} />
    {onClear && props.value && (
      <button type='button' className='kit-input-clear' aria-label='Очистить поиск' onClick={onClear}>
        <Icon name='X' size={16} />
      </button>
    )}
    {submit ? (
      <button type='submit' aria-label='Найти' disabled={props.disabled}><Icon name='Search' size={20} /></button>
    ) : icon && <Icon name={icon} size={20} />}
  </div>
));

Input.displayName = 'Input';
