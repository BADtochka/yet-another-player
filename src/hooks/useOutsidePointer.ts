import type { RefObject } from 'preact';
import { useEffect } from 'preact/hooks';

export function useOutsidePointer(
  root: RefObject<HTMLElement>,
  open: boolean,
  onDismiss: () => void,
  trigger?: RefObject<HTMLElement>,
) {
  useEffect(() => {
    if (!open) return;
    let startedOutside = false;
    const outside = (target: EventTarget | null) => target instanceof Element
      && !root.current?.contains(target) && !trigger?.current?.contains(target)
      && !target.closest('dialog[open], [popover]:popover-open');
    const handlePointer = (event: PointerEvent) => {
      startedOutside = outside(event.target);
    };
    const handleClick = (event: MouseEvent) => {
      if ((startedOutside || event.detail === 0) && outside(event.target)) onDismiss();
      startedOutside = false;
    };
    document.addEventListener('pointerdown', handlePointer);
    document.addEventListener('click', handleClick, true);
    return () => {
      document.removeEventListener('pointerdown', handlePointer);
      document.removeEventListener('click', handleClick, true);
    };
  }, [root, open, onDismiss, trigger]);
}
