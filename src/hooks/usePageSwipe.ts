import { useRouter, useRouterState } from '@tanstack/react-router';
import { listen } from '@tauri-apps/api/event';
import type { RefObject } from 'preact';
import { useEffect, useLayoutEffect, useRef } from 'preact/hooks';

const SWIPE_GAP_MS = 120;
const SWIPE_DISTANCE = 76;
const SWIPE_PREVIEW_PROGRESS = 0.6;
const SWIPE_CONFIRM_MS = 100;
const SWIPE_COMMIT_DELAY_MS = 160;
const HORIZONTAL_RATIO = 1.6;

type Gesture = {
  horizontal: number;
  vertical: number;
  index: number;
  committed: boolean;
  lastDelta: number;
};

type NativeSwipe =
  | { phase: 'preview' | 'commit'; x: number; y: number; back: boolean }
  | { phase: 'cancel' };

function usesWheelLocally(target: Element | null, surface: HTMLElement) {
  if (!target || !surface.contains(target) || target.closest('dialog, [popover], select')) return true;
  for (let node: Element | null = target; node && node !== surface; node = node.parentElement) {
    if (node.scrollWidth > node.clientWidth + 1
      && /^(auto|scroll|overlay)$/.test(getComputedStyle(node).overflowX)) return true;
  }
  return false;
}

export function usePageSwipe(surface: RefObject<HTMLElement>, page: RefObject<HTMLElement>, enabled: boolean, native: boolean) {
  const { history } = useRouter();
  const pageIndex = useRouterState({ select: (state) => state.resolvedLocation?.state.__TSR_index });
  const feedback = useRef<HTMLDivElement>(null);
  const lastPage = useRef(history.location.state.__TSR_index);
  const pending = useRef<{ index: number; step: number; started: number }>();

  useEffect(() => {
    if (!enabled) return;
    lastPage.current = history.location.state.__TSR_index;
    return history.subscribe(({ location, action }) => {
      const index = location.state.__TSR_index;
      lastPage.current = action.type === 'PUSH' ? index : Math.max(lastPage.current, index);
      if (pending.current?.index !== index) pending.current = undefined;
    });
  }, [enabled, history]);

  useEffect(() => {
    const element = surface.current;
    const indicator = feedback.current;
    if (!element || !indicator || !enabled) return;

    let gesture: Gesture | undefined;
    let settleTimer: number | undefined;
    let commitTimer: number | undefined;
    let retractTimer: number | undefined;
    let nativeSwipeIndex: number | undefined;

    const retract = () => {
      delete indicator.dataset.direction;
      delete indicator.dataset.committed;
      indicator.style.removeProperty('--page-swipe-progress');
    };

    const direction = (horizontal: number) => horizontal > 0 ? 1 : -1;
    const canNavigate = (step: number, index: number) =>
      step < 0 ? history.canGoBack() : index < lastPage.current;

    const showProgress = (step: number, progress: number) => {
      window.clearTimeout(retractTimer);
      delete indicator.dataset.committed;
      indicator.dataset.direction = step < 0 ? 'back' : 'forward';
      indicator.style.setProperty('--page-swipe-progress', String(progress));
    };

    const navigate = (step: number, current: number) => {
      showProgress(step, 1);
      indicator.setAttribute('data-committed', '');
      retractTimer = window.setTimeout(retract, SWIPE_CONFIRM_MS);
      pending.current = { index: current + step, step, started: performance.now() };
      if (step < 0) history.back();
      else history.forward();
    };

    const commit = (finished: Gesture) => {
      window.clearTimeout(commitTimer);
      commitTimer = undefined;
      const current = history.location.state.__TSR_index;
      if (finished.committed || finished.index !== current
        || Math.abs(finished.horizontal) < Math.max(SWIPE_DISTANCE, finished.vertical * HORIZONTAL_RATIO)
        || !canNavigate(direction(finished.horizontal), current)) {
        retract();
        return;
      }

      finished.committed = true;
      navigate(direction(finished.horizontal), current);
    };

    const finish = () => {
      if (gesture && !gesture.committed) commit(gesture);
      gesture = undefined;
    };

    const reset = () => {
      gesture = undefined;
      nativeSwipeIndex = undefined;
      window.clearTimeout(settleTimer);
      window.clearTimeout(commitTimer);
      commitTimer = undefined;
      window.clearTimeout(retractTimer);
      retract();
    };

    const onVisibilityChange = () => {
      if (document.hidden) reset();
    };

    const onWheel = (event: WheelEvent) => {
      if (event.defaultPrevented || event.deltaMode !== WheelEvent.DOM_DELTA_PIXEL || event.ctrlKey
        || event.shiftKey || event.altKey || event.metaKey) return;

      const vertical = Math.abs(event.deltaY);
      const horizontal = Math.abs(event.deltaX);
      if (gesture?.committed && horizontal >= Math.max(12, vertical * HORIZONTAL_RATIO)
        && gesture.lastDelta <= 4) reset();

      if (!gesture) {
        if (horizontal < 1 || horizontal < vertical * HORIZONTAL_RATIO) return;
        const target = document.elementFromPoint(event.clientX, event.clientY)
          ?? (event.target instanceof Element ? event.target : null);
        if (usesWheelLocally(target, element)) return;
        window.clearTimeout(retractTimer);
        retract();
        gesture = {
          horizontal: 0,
          vertical: 0,
          index: history.location.state.__TSR_index,
          committed: false,
          lastDelta: horizontal,
        };
      }
      if (horizontal >= vertical && event.cancelable) event.preventDefault();

      // Drain the momentum events after a committed swipe, until input goes quiet.
      window.clearTimeout(settleTimer);
      settleTimer = window.setTimeout(finish, SWIPE_GAP_MS);
      gesture.lastDelta = horizontal;
      if (gesture.committed || horizontal + vertical < 1) return;

      gesture.horizontal += event.deltaX;
      gesture.vertical += vertical;
      const distance = Math.abs(gesture.horizontal);
      const step = direction(gesture.horizontal);
      if (distance === 0 || distance < gesture.vertical * HORIZONTAL_RATIO || !canNavigate(step, gesture.index)) {
        window.clearTimeout(commitTimer);
        commitTimer = undefined;
        retract();
        return;
      }
      showProgress(step, Math.max(SWIPE_PREVIEW_PROGRESS, Math.min(distance / SWIPE_DISTANCE, 1)));
      if (distance >= SWIPE_DISTANCE && commitTimer === undefined) {
        const swipe = gesture;
        commitTimer = window.setTimeout(() => commit(swipe), SWIPE_COMMIT_DELAY_MS);
      }
    };

    let active = true;
    const startWheelFallback = () => {
      if (active) window.addEventListener('wheel', onWheel, { capture: true, passive: false });
    };
    const nativeListener = native ? listen<NativeSwipe>('page-swipe', ({ payload }) => {
      if (!active || document.hidden) return;
      if (payload.phase === 'cancel') {
        nativeSwipeIndex = undefined;
        retract();
        return;
      }
      const current = history.location.state.__TSR_index;
      const step = payload.back ? -1 : 1;
      const samePage = nativeSwipeIndex === undefined || nativeSwipeIndex === current;
      const available = samePage && canNavigate(step, current)
        && !usesWheelLocally(document.elementFromPoint(payload.x, payload.y), element);
      nativeSwipeIndex = payload.phase === 'preview' ? nativeSwipeIndex ?? current : undefined;
      if (!available) retract();
      else if (payload.phase === 'preview') showProgress(step, SWIPE_PREVIEW_PROGRESS);
      else navigate(step, current);
    }).catch(startWheelFallback) : undefined;
    if (!native) startWheelFallback();
    window.addEventListener('blur', reset);
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      active = false;
      void nativeListener?.then((unlisten) => unlisten?.());
      window.removeEventListener('wheel', onWheel, true);
      window.removeEventListener('blur', reset);
      document.removeEventListener('visibilitychange', onVisibilityChange);
      reset();
      pending.current = undefined;
    };
  }, [enabled, history, native, surface]);

  useLayoutEffect(() => {
    const element = page.current;
    const target = pending.current;
    if (!element || !target || target.index !== pageIndex) return;
    pending.current = undefined;
    if (performance.now() - target.started > 1500 || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const animation = element.animate([
      { transform: `translateX(${target.step * 40}px)`, opacity: 0.9 },
      { transform: 'translateX(0)', opacity: 1 },
    ], { duration: 230, easing: 'cubic-bezier(.2, .7, .25, 1)' });
    return () => animation.cancel();
  }, [pageIndex, page]);

  return feedback;
}
