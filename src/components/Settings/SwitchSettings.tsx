import type { ComponentChildren } from 'preact';
import { BetaBadge } from '@/components/Settings/BetaBadge';
import { Switch } from '@/components/Switch';

export function SwitchSettings({ label, beta, children, checked, onChange }: {
  label: string;
  beta?: boolean;
  checked: boolean;
  onChange: (value: boolean) => void;
  children?: ComponentChildren;
}) {
  return (
    <div className='settings-switch'>
      <div className='settings-row'>
        <p>{label}{beta && <BetaBadge />}</p>
        <Switch label={label} checked={checked} onChange={onChange} />
      </div>
      {checked && children}
    </div>
  );
}
