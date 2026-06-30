import { Link } from '@tanstack/react-router';
import { cn } from 'badlib';
import type { ComponentChildren } from 'preact';
import { FileRouteTypes } from '@/routeTree.gen';

type TabProps = {
  children: ComponentChildren;
  icon: ComponentChildren;
  to: FileRouteTypes['to'];
  className?: string;
};

export const Tab = ({ children, icon, to, className }: TabProps) => {
  return (
    <Link
      to={to}
      className={cn(
        'inline-flex min-h-12 items-center justify-center gap-2.5 rounded-lg border border-transparent bg-[#020a1d] px-6 text-base/none font-medium whitespace-nowrap text-white no-underline transition-colors duration-150 hover:bg-[#061232] data-status:border-[#405EA4]',
        className,
      )}
    >
      <span className='inline-flex size-5 shrink-0 [&>svg]:size-5' aria-hidden='true'>
        {icon}
      </span>
      <span>{children}</span>
    </Link>
  );
};
