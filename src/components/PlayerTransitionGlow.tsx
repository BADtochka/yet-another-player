import type { RefObject } from 'preact';
import { useLayoutEffect, useRef } from 'preact/hooks';

const TRAVEL_MS = 350;
const FADE_START_MS = TRAVEL_MS / 2;
const DURATION_MS = 500;
const INSET = 3;

const ease = (progress: number) => progress ** 2 * (3 - 2 * progress);

export function PlayerTransitionGlow({ anchor, onComplete }: {
  anchor: RefObject<HTMLButtonElement>;
  onComplete: () => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);

  useLayoutEffect(() => {
    const surface = canvas.current;
    const context = surface?.getContext('2d');
    if (!surface || !context) {
      onComplete();
      return;
    }

    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
    const color = getComputedStyle(surface).getPropertyValue('--yap-focus').trim();
    let width = 0;
    let height = 0;
    let perimeter = 0;
    let path = new Path2D();

    const resize = () => {
      width = surface.clientWidth;
      height = surface.clientHeight;
      const scale = Math.min(window.devicePixelRatio || 1, 2);
      surface.width = Math.round(width * scale);
      surface.height = Math.round(height * scale);
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.lineCap = 'round';
      context.lineWidth = 3;
      context.strokeStyle = color;
      const left = INSET;
      const top = INSET;
      const right = width - INSET;
      const bottom = height - INSET;
      const radius = Math.max(0, Math.min(10, (right - left) / 2, (bottom - top) / 2));
      const button = anchor.current?.getBoundingClientRect();
      const x = button ? button.left + button.width / 2 : width / 2;
      const start = Math.min(right - radius, Math.max(left + radius, x));
      path = new Path2D();
      path.moveTo(start, bottom);
      path.arcTo(left, bottom, left, top, radius);
      path.arcTo(left, top, right, top, radius);
      path.arcTo(right, top, right, bottom, radius);
      path.arcTo(right, bottom, start, bottom, radius);
      path.closePath();
      perimeter = 2 * (right - left + bottom - top) + (2 * Math.PI - 8) * radius;
    };

    resize();
    window.addEventListener('resize', resize);
    let started: number | undefined;
    let frame: number;
    const draw = (now: number) => {
      started ??= now;
      const elapsed = now - started;
      if (reducedMotion.matches || elapsed >= DURATION_MS) {
        onComplete();
        return;
      }
      context.clearRect(0, 0, width, height);
      const fadeProgress = Math.max(0, (elapsed - FADE_START_MS) / (DURATION_MS - FADE_START_MS));
      context.globalAlpha = 1 - ease(fadeProgress);
      if (elapsed >= TRAVEL_MS) {
        context.setLineDash([]);
        context.stroke(path);
      } else if (elapsed > 0) {
        const length = ease(elapsed / TRAVEL_MS) * perimeter;
        // Center the visible stroke on the origin of the closed path.
        context.setLineDash([length, perimeter - length]);
        context.lineDashOffset = length / 2;
        context.stroke(path);
      }
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener('resize', resize);
    };
  }, [anchor, onComplete]);

  return <canvas ref={canvas} className='player-transition-glow' aria-hidden='true' />;
}
