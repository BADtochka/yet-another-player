import { cn } from 'badlib';
import { ComponentChildren } from 'preact';

type SettingsTextActionProps = {
  children: ComponentChildren;
  className?: string;
  disabled?: boolean;
  onClick?: () => void;
};

export const SettingsTextAction = ({ children, className, disabled = false, onClick }: SettingsTextActionProps) => {
  return (
    <button
      type='button'
      disabled={disabled}
      onClick={onClick}
      className={cn(
        'text-base font-medium text-[#506ba9] disabled:cursor-not-allowed disabled:opacity-50 w-fit',
        className,
      )}
    >
      {children}
    </button>
  );
};
