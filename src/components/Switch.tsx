import { cn } from 'badlib';

export type SwitchProps = {
  checked: boolean;
  label: string;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  className?: string;
  id?: string;
};

export function Switch({ checked, label, onChange, disabled, className, id }: SwitchProps) {
  return (
    <button id={id} type='button' role='switch' aria-label={label} aria-checked={checked}
      className={cn('kit-switch', className)} disabled={disabled} onClick={() => onChange?.(!checked)}>
      <span aria-hidden='true' />
    </button>
  );
}
