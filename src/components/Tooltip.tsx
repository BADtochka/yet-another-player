import { useLayoutEffect, useRef } from 'preact/hooks';

export function Tooltip({ label, placement = 'right' }: { label: string; placement?: 'right' | 'top' }) {
  const ref = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    const tooltip = ref.current;
    const anchor = tooltip?.parentElement;
    if (placement !== 'top' || !tooltip || !anchor) return;

    const align = () => {
      const { width } = tooltip.getBoundingClientRect();
      if (!width) return;
      const button = anchor.getBoundingClientRect();
      const centered = button.left + (button.width - width) / 2;
      const rightLimit = document.documentElement.clientWidth - width;
      const left = Math.max(0, Math.min(centered, rightLimit));
      tooltip.style.setProperty('--kit-tooltip-shift', `${left - centered}px`);
    };

    const resize = new ResizeObserver(align);
    resize.observe(tooltip);
    resize.observe(anchor);
    window.addEventListener('resize', align);
    align();
    return () => {
      resize.disconnect();
      window.removeEventListener('resize', align);
    };
  }, [placement]);

  return <span ref={ref} className='kit-tooltip' data-placement={placement} aria-hidden='true'>{label}</span>;
}
