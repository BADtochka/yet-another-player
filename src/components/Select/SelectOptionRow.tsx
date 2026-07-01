import { cn } from 'badlib';
import { type ComponentChildren } from 'preact';

export type SelectOptionRowProps = {
  children: ComponentChildren;
  selected: boolean;
  onSelect: () => void;
};

export const SelectOptionRow = ({ children, selected, onSelect }: SelectOptionRowProps) => (
  <div
    role='option'
    aria-selected={selected}
    onClick={onSelect}
    className={cn(
      'flex w-full cursor-pointer items-center p-2.5 text-base/none font-medium text-white',
      selected && 'bg-[#061232]',
    )}
  >
    {children}
  </div>
);
