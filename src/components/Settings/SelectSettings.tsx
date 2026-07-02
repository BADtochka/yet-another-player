import { Select, type SelectProps } from '@/components/Select';
import { motion } from 'motion/react';
import { ComponentChildren } from 'preact';

export type SelectSettingsProps<T extends string = string> = {
  label: string;
  children?: ComponentChildren;
  open?: boolean;
} & SelectProps<T>;

export const SelectSettings = <T extends string = string>({
  label,
  children,
  open = true,
  className,
  ...selectProps
}: SelectSettingsProps<T>) => {
  return (
    <motion.div
      className='flex items-center justify-between gap-4'
      initial={false}
      animate={{
        gap: open ? 10 : 0,
        height: open ? 'auto' : 0,
      }}
    >
      <p>{label}</p>
      <Select {...selectProps} className={className ?? 'w-36 shrink-0'} />
    </motion.div>
  );
};
