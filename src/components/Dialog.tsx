import { cn } from 'badlib';
import type { ComponentChildren } from 'preact';
import { useLayoutEffect, useId, useRef } from 'preact/hooks';
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
  const pressedBackdrop = useRef(false);
  const titleId = useId();
  const descriptionId = useId();

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    pressedBackdrop.current = false;
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
      onPointerDown={(event) => {
        const { left, top, right, bottom } = event.currentTarget.getBoundingClientRect();
        pressedBackdrop.current = event.target === event.currentTarget
          && (event.clientX < left || event.clientX >= right || event.clientY < top || event.clientY >= bottom);
      }}
      onPointerCancel={() => { pressedBackdrop.current = false; }}
      onClick={(event) => {
        const startedOutside = pressedBackdrop.current;
        pressedBackdrop.current = false;
        if (!startedOutside || busy || event.target !== event.currentTarget) return;
        const { left, top, right, bottom } = event.currentTarget.getBoundingClientRect();
        if (event.clientX < left || event.clientX >= right || event.clientY < top || event.clientY >= bottom) onDismiss();
      }}
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
