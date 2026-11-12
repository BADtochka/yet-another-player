import { cn } from 'badlib';

export function Progress({ label, value, max, counts = false, className }: {
  label: string;
  value?: number | null;
  max?: number | null;
  counts?: boolean;
  className?: string;
}) {
  return (
    <div className={cn('kit-progress', className)}>
      {counts && <span aria-hidden='true'>{value ?? 0}</span>}
      <progress aria-label={label} max={max || 1} value={max ? value ?? undefined : undefined} />
      {counts && <span aria-hidden='true'>{max ?? '…'}</span>}
    </div>
  );
}
