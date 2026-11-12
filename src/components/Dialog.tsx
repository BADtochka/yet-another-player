import { cn } from 'badlib';
import type { ComponentChildren } from 'preact';
import { useEffect, useId, useRef } from 'preact/hooks';
import { Button } from '@/components/Button';

export function Dialog({ open, title, description, busy, className, onDismiss, children }: {
  open: boolean;
  title: string;
  description?: ComponentChildren;
  busy?: boolean;
  className?: string;
  onDismiss: () => void;
  children: ComponentChildren;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    if (open && !element.open) {
      element.showModal();
      element.querySelector<HTMLElement>('[data-initial-focus]')?.focus();
    } else if (!open && element.open) {
      element.close();
    }
  }, [open]);

  return (
    <dialog ref={ref} className={cn('kit-dialog', className)} aria-labelledby={titleId}
      aria-describedby={description ? descriptionId : undefined}
      onCancel={(event) => { event.preventDefault(); if (!busy) onDismiss(); }}>
      <div className='kit-dialog-content'>
        <header className='kit-dialog-heading'>
          <h2 id={titleId}>{title}</h2>
          <Button variant='ghost' size='icon' icon='X' aria-label='Закрыть' disabled={busy} onClick={onDismiss} />
        </header>
        {description && <p id={descriptionId}>{description}</p>}
        {children}
      </div>
    </dialog>
  );
}
