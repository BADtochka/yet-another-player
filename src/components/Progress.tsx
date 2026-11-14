import { cn } from 'badlib';

export function Progress({ label, value, max, counts = false, className }: {
  label: string;
  value?: number | null;
  max?: number | null;
  counts?: boolean;
  className?: string;
}) {
  const measured = typeof max === 'number' && max > 0;
  return (
    <div className={cn('kit-progress', className)}>
      {counts && measured && <span aria-hidden='true'>{value ?? 0}</span>}
      <progress aria-label={label} max={measured ? max : undefined} value={measured ? value ?? undefined : undefined} />
      {counts && measured && <span aria-hidden='true'>{max}</span>}
    </div>
  );
}
