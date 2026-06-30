import { ComponentChildren } from 'preact';
import { useState } from 'preact/hooks';
import { Switch } from '../Switch';

type SwitchSettingsProps = {
  label: string;
  children?: ComponentChildren;
};

export const SwitchSettings = ({ label, children }: SwitchSettingsProps) => {
  const [_devChecked, _setDevChecked] = useState(false);

  return (
    <div className='flex flex-col gap-2.5'>
      <div className='flex items-center justify-between'>
        <p>{label}</p>
        <Switch checked={_devChecked} onChange={_setDevChecked} />
      </div>
      {children}
    </div>
  );
};
