import type { ComponentChildren } from 'preact';
import { Switch } from '@/components/Switch';

export function SwitchSettings({ label, children, checked, onChange }: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  children?: ComponentChildren;
}) {
  return (
    <div className='settings-switch'>
      <div className='settings-row'>
        <p>{label}</p>
        <Switch label={label} checked={checked} onChange={onChange} />
      </div>
      {checked && children}
    </div>
  );
}
