import { cn } from 'badlib';

export function Spinner({ size }: { size?: number }) {
  return <span className='kit-spinner' style={size ? { width: size, height: size } : undefined} aria-hidden='true' />;
}

export function LoadingIndicator({ label, className }: { label: string; className?: string }) {
  return (
    <span className={cn('kit-loading-indicator', className)} role='status' aria-label={label}>
      <Spinner />
    </span>
  );
}
