import { cn } from 'badlib';
import { useEffect, useId, useRef, useState } from 'preact/hooks';

import { Minus } from '@/components/Icons/Minus';
import { Next } from '@/components/Icons/Next';
import { SelectOptionRow } from '@/components/Select/SelectOptionRow';

export type SelectOption<T extends string = string> = {
  value: T;
  label: string;
};

export type SelectProps<T extends string = string> = {
  value: T;
  options: readonly SelectOption<T>[];
  onChange?: (value: T) => void;
  disabled?: boolean;
  className?: string;
  id?: string;
  placeholder?: string;
};

export const Select = <T extends string = string>({
  value,
  options,
  onChange,
  disabled = false,
  className,
  id,
  placeholder = 'Выберите значение',
}: SelectProps<T>) => {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const listboxId = useId();
  const selectedOption = options.find((option) => option.value === value);
  const label = selectedOption?.label ?? placeholder;

  useEffect(() => {
    if (!open) {
      return;
    }

    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };

    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);

    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  const onToggle = () => {
    if (disabled) {
      return;
    }

    setOpen((current) => !current);
  };

  const onSelect = (nextValue: T) => {
    if (disabled) {
      return;
    }

    onChange?.(nextValue);
    setOpen(false);
  };

  return (
    <div ref={rootRef} id={id} className={cn('relative inline-flex w-full flex-col', className)}>
      <div
        role='button'
        tabIndex={disabled ? -1 : 0}
        aria-expanded={open}
        aria-haspopup='listbox'
        aria-controls={listboxId}
        onClick={onToggle}
        onKeyDown={(event) => {
          if (disabled) {
            return;
          }

          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            onToggle();
          }
        }}
        className={cn(
          'flex w-full items-center justify-between gap-2.5 rounded-lg bg-[#020a1d] p-2.5',
          disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer',
        )}
      >
        <span className='py-[7px] text-base/none font-medium text-[#d9d9d9]'>{label}</span>
        <span className='inline-flex size-6 shrink-0 items-center justify-center text-[#a3a5aa]'>
          {open ? <Next size={16} aria-hidden /> : <Minus size={16} className='rotate-90' aria-hidden />}
        </span>
      </div>

      {open ? (
        <div
          id={listboxId}
          role='listbox'
          className='absolute top-full right-0 left-0 z-20 mt-0 flex flex-col border border-[#414141] bg-[#020a1d]'
        >
          {options.map((option) => (
            <SelectOptionRow
              key={option.value}
              selected={option.value === value}
              onSelect={() => onSelect(option.value)}
            >
              {option.label}
            </SelectOptionRow>
          ))}
        </div>
      ) : null}
    </div>
  );
};
