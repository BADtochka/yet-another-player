import { motion } from 'motion/react';
import { ComponentChildren } from 'preact';
import { Switch } from '../Switch';

type SwitchSettingsProps = {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  children?: ComponentChildren;
};

export const SwitchSettings = ({ label, children, checked, onChange }: SwitchSettingsProps) => {
  return (
    <motion.div className='flex flex-col' initial={false} animate={{ gap: checked ? 10 : 0 }}>
      <div className='flex items-center justify-between'>
        <p>{label}</p>
        <Switch checked={checked} onChange={onChange} />
      </div>
      <motion.div
        initial={false}
        animate={{
          y: Number(checked),
          height: checked ? 'auto' : 0,
          opacity: Number(checked),
        }}
        className='z-20'
      >
        {children}
      </motion.div>
    </motion.div>
  );
};
