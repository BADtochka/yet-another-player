import { Link } from '@tanstack/react-router';
import { cn } from 'badlib';
import { Icon, type IconName } from '@/components/Icons/Icon';
import { Tooltip } from '@/components/Tooltip';
import type { FileRouteTypes } from '@/routeTree.gen';

export function Tab({ label, icon, to, className }: {
  label: string;
  icon: IconName;
  to: FileRouteTypes['to'];
  className?: string;
}) {
  return (
    <Link to={to} aria-label={label} activeOptions={{ exact: true }} className={cn('app-nav-link', className)}>
      <Icon name={icon} />
      <Tooltip label={label} />
    </Link>
  );
}
