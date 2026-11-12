import { cn } from 'badlib';
import { useEffect, useId, useRef, useState } from 'preact/hooks';
import { Icon } from '@/components/Icons/Icon';

export type SelectOption<T extends string = string> = {
  value: T;
  label: string;
};

export type SelectProps<T extends string = string> = {
  label: string;
  value: T;
  options: readonly SelectOption<T>[];
  onChange?: (value: T) => void;
  disabled?: boolean;
  className?: string;
  id?: string;
  placeholder?: string;
};

export function Select<T extends string>({
  label, value, options, onChange, disabled, className, id, placeholder = 'Выберите значение',
}: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const selected = options.findIndex((option) => option.value === value);
  const close = () => setOpen(false);

  useEffect(() => {
    if (!open) return;
    const onOutside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) close();
    };
    document.addEventListener('pointerdown', onOutside);
    return () => document.removeEventListener('pointerdown', onOutside);
  }, [open]);

  const show = () => {
    trigger.current?.focus({ preventScroll: true });
    setActive(Math.max(0, selected));
    setOpen(true);
  };
  const choose = (option: SelectOption<T>) => {
    onChange?.(option.value);
    close();
    trigger.current?.focus();
  };

  return (
    <div ref={root} className={cn('kit-select', className)} data-open={open}
      onFocusOut={(event) => { if (!root.current?.contains(event.relatedTarget as Node | null)) close(); }}>
      <button ref={trigger} id={id} type='button' role='combobox' aria-label={label} aria-expanded={open}
        aria-haspopup='listbox' aria-controls={listId} aria-activedescendant={open ? `${listId}-${active}` : undefined}
        disabled={disabled || !options.length} onClick={open ? close : show}
        onKeyDown={(event) => {
          const { key } = event;
          if (key === 'Escape' && open) { event.preventDefault(); close(); return; }
          if (open && (key === 'Enter' || key === ' ')) {
            event.preventDefault();
            const option = options[active];
            if (option) choose(option);
            return;
          }
          const index = { ArrowDown: active + 1, ArrowUp: active - 1, Home: 0, End: options.length - 1 }[key];
          if (index === undefined) return;
          event.preventDefault();
          if (!open) show();
          else setActive(Math.min(options.length - 1, Math.max(0, index)));
        }}>
        <span>{options[selected]?.label ?? placeholder}</span>
        <Icon name={open ? 'ChevronUp' : 'ChevronDown'} size={16} />
      </button>
      {open && (
        <ul id={listId} role='listbox' aria-label={label}>
          {options.map((option, index) => (
            <li id={`${listId}-${index}`} key={option.value} role='option' aria-selected={index === selected}
              data-active={index === active} onPointerDown={(event) => event.preventDefault()}
              onClick={() => choose(option)}>
              {option.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
