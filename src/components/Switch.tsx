import { cn } from 'badlib';
import { motion, type Variants } from 'motion/react';

export type SwitchProps = {
  checked: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  className?: string;
  id?: string;
};

export const Switch = ({ checked, onChange, disabled = false, className, id }: SwitchProps) => {
  const onToggle = () => {
    if (disabled) {
      return;
    }

    onChange?.(!checked);
  };

  const KNOB_VARIANTS: Variants = {
    unchecked: {
      x: 0,
      width: 20,
      height: 20,
      scale: 1,
    },
    checked: {
      x: 23,
      width: 20,
      height: 20,
      scale: 1,
    },
  };

  return (
    <div
      id={id}
      onClick={onToggle}
      className={cn(
        'inline-flex flex-col gap-2.5 rounded-[5px] p-2.5 transition-opacity',
        disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer',
        className,
      )}
    >
      <span
        className={cn(
          'relative h-6 w-[47px] rounded-[11px] transition-colors duration-150',
          checked ? 'bg-[#091d51]' : 'bg-[#020a1d]',
        )}
      >
        <motion.div
          variants={KNOB_VARIANTS}
          animate={checked ? 'checked' : 'unchecked'}
          custom={checked}
          transition={{ type: 'spring', stiffness: 500, damping: 30, duration: 0.3 }}
          className='absolute top-0.5 left-0.5 rounded-full bg-[#d9d9d9]'
        />
      </span>
    </div>
  );
};
