import { useCallback, useLayoutEffect, useRef, useState } from 'preact/hooks';
import { Icon } from '@/components/Icons/Icon';
import { PlayerTransitionGlow } from '@/components/PlayerTransitionGlow';
import { Tooltip } from '@/components/Tooltip';
import { useSettingsStore } from '@/stores/settingsStore';
import type { TransitionSettings } from '@/types/Settings';

const isAdvanced = ({ enabled, mode }: TransitionSettings) => enabled && mode === 'advanced';

export function AdvancedTransitionsButton({ ready }: { ready: boolean }) {
  const mode = useSettingsStore(({ transitions }) => transitions.enabled ? transitions.mode : null);
  const advanced = mode === 'advanced';
  const button = useRef<HTMLButtonElement>(null);
  const sequence = useRef(0);
  const [glow, setGlow] = useState<number>();
  const clearGlow = useCallback(() => setGlow(undefined), []);

  useLayoutEffect(() => {
    if (!ready) return;
    return useSettingsStore.subscribe((state, previous) => {
      const enabled = isAdvanced(state.transitions);
      if (enabled === isAdvanced(previous.transitions)) return;
      if (!enabled) {
        clearGlow();
        return;
      }
      if (!window.matchMedia('(prefers-reduced-motion: reduce)').matches) setGlow(++sequence.current);
    });
  }, [ready, clearGlow]);

  if (!ready || !mode) return null;

  return <>
    <button ref={button} type='button' className='player-tool player-advanced-transitions'
      aria-label='Продвинутые переходы'
      aria-pressed={advanced}
      onClick={() => {
        const { transitions, setTransitions } = useSettingsStore.getState();
        setTransitions({ mode: transitions.mode === 'advanced' ? 'smooth' : 'advanced' });
      }}>
      <Icon className='player-transition-icon' name='Wave' data-mode='smooth' />
      <Icon className='player-transition-icon' name='Waveform' data-mode='advanced' />
      <Tooltip label={advanced ? 'Плавные переходы' : 'Продвинутые переходы'} placement='top' />
    </button>
    {glow !== undefined && <PlayerTransitionGlow key={glow} anchor={button} onComplete={clearGlow} />}
  </>;
}
