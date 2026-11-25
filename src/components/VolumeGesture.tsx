import { invoke } from '@tauri-apps/api/core';
import { useEffect } from 'preact/hooks';
import { useSettingsStore } from '@/stores/settingsStore';
import { activeVolumeGesture, canRetryVolumeGestureAccess, configureVolumeGesture, modifierMask, useVolumeGesture } from '@/stores/volumeGestureStore';
import { wheelVolumeChange } from '@/utils/volumeWheel';

const mouseButtons: Record<string, number> = { MouseMiddle: 4, MouseBack: 8, MouseForward: 16 };

export function VolumeGesture() {
  useEffect(() => {
    configureVolumeGesture(activeVolumeGesture(useSettingsStore.getState()));
    const unsubscribe = useSettingsStore.subscribe((state, previous) => {
      const binding = activeVolumeGesture(state);
      if (binding !== activeVolumeGesture(previous)) configureVolumeGesture(binding);
    });
    const keys = new Set<string>();
    const clearKeys = () => keys.clear();
    const keyDown = (event: KeyboardEvent) => keys.add(event.code);
    const keyUp = (event: KeyboardEvent) => keys.delete(event.code);
    const refreshAccess = () => {
      const { status, pending } = useVolumeGesture.getState();
      if (pending || !canRetryVolumeGestureAccess(status)) return;
      const binding = activeVolumeGesture(useSettingsStore.getState());
      if (binding) configureVolumeGesture(binding);
    };
    const wheel = (event: WheelEvent) => {
      const { status, recording, pending } = useVolumeGesture.getState();
      if (status === 'off' || status === 'global' || recording || pending) return;
      const binding = activeVolumeGesture(useSettingsStore.getState());
      if (!binding || binding.modifiers !== modifierMask(event)) return;
      if (binding.key) {
        const button = mouseButtons[binding.key];
        if (button ? !(event.buttons & button) : !keys.has(binding.key)) return;
      }
      const delta = wheelVolumeChange(event);
      if (!delta) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      // X11 handles the same scroll independently of the webview.
      if (status !== 'x11') void invoke('volume_gesture_scroll', { delta }).catch(() => undefined);
    };
    window.addEventListener('keydown', keyDown, true);
    window.addEventListener('keyup', keyUp, true);
    window.addEventListener('blur', clearKeys);
    window.addEventListener('focus', refreshAccess);
    document.addEventListener('visibilitychange', clearKeys);
    window.addEventListener('wheel', wheel, { capture: true, passive: false });
    return () => {
      unsubscribe();
      window.removeEventListener('keydown', keyDown, true);
      window.removeEventListener('keyup', keyUp, true);
      window.removeEventListener('blur', clearKeys);
      window.removeEventListener('focus', refreshAccess);
      document.removeEventListener('visibilitychange', clearKeys);
      window.removeEventListener('wheel', wheel, true);
    };
  }, []);

  return null;
}
