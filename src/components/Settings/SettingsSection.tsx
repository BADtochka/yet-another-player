import { cn } from 'badlib';
import { ComponentChildren } from 'preact';

type SettingsSectionProps = {
  title: string;
  description?: string;
  children?: ComponentChildren;
  className?: string;
};

export const SettingsSection = ({ title, description, children, className }: SettingsSectionProps) => {
  return (
    <section className={cn('flex flex-col gap-5 rounded-[5px] justify-start', className)}>
      <div className='flex flex-col'>
        <h2 className='text-[32px] font-medium leading-none text-[#d9d9d9]'>{title}</h2>
        {description && <p className='text-base font-medium text-[#d9d9d9]'>{description}</p>}
      </div>
      {children}
    </section>
  );
};
