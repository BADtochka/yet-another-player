import { cn } from 'badlib';
import type { JSX } from 'preact';
import { forwardRef } from 'preact/compat';
import { Search } from '@/components/Icons/Search';

type InputProps = Omit<JSX.InputHTMLAttributes<HTMLInputElement>, 'size'>;

export const Input = forwardRef<HTMLInputElement, InputProps>(({ className, placeholder, ...props }, ref) => {
  return (
    <label
      className={cn(
        'flex min-h-12 w-full items-center justify-between gap-2.5 rounded-lg border border-transparent bg-[#020a1d] p-2.5 text-[#516ba9] transition-colors duration-150 focus-within:border-[#405ea4] focus-within:bg-[#061232] focus-within:text-white',
        className,
      )}
    >
      <input
        ref={ref}
        placeholder={placeholder}
        className='min-w-0 grow border-0 bg-transparent p-0 font-onest text-base/none font-medium text-inherit outline-none placeholder:text-[#516ba9]'
        {...props}
      />
      <Search size={20} className='shrink-0 text-current' aria-hidden='true' />
    </label>
  );
});

Input.displayName = 'Input';
