import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import type { Settings, VolumeGestureBinding } from '@/types/Settings';

export type GestureStatus = 'off' | 'global' | 'x11' | 'wayland' | 'local'
  | 'permissionRequired' | 'macInterceptionUnavailable';
export const activeVolumeGesture = (state: Pick<Settings, 'volumeGesture' | 'volumeGestureEnabled'>) =>
  state.volumeGestureEnabled === false ? null : state.volumeGesture;
export const canRetryVolumeGestureAccess = (status: GestureStatus) =>
  status === 'permissionRequired' || status === 'macInterceptionUnavailable';

export const useVolumeGesture = create<{
  status: GestureStatus;
  pending: boolean;
  error: string;
  recording: boolean;
}>(() => ({ status: 'off', pending: false, error: '', recording: false }));

let request = 0;
let configuration = Promise.resolve();
let configuredBinding: VolumeGestureBinding | null = null;

export function configureVolumeGesture(binding: VolumeGestureBinding | null, requestPermission = false) {
  const revision = ++request;
  const status = binding === configuredBinding ? useVolumeGesture.getState().status : 'off';
  configuredBinding = binding;
  useVolumeGesture.setState({ status, pending: true, error: '' });
  configuration = configuration.then(async () => {
    if (revision !== request) return;
    try {
      const status = await invoke<GestureStatus>('configure_volume_gesture', { binding, requestPermission });
      if (revision === request) useVolumeGesture.setState({ status, pending: false });
    } catch (reason) {
      if (revision === request) useVolumeGesture.setState({ pending: false, error: String(reason) });
    }
  });
}

export function modifierMask(event: Pick<KeyboardEvent, 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey'>) {
  return Number(event.ctrlKey) | Number(event.shiftKey) << 1 | Number(event.altKey) << 2 | Number(event.metaKey) << 3;
}

export const mouseBinding = (button: number) => ({ 1: 'MouseMiddle', 3: 'MouseBack', 4: 'MouseForward' }[button]);

const labels: Record<string, string> = {
  Space: 'Пробел', MouseMiddle: 'Средняя кнопка мыши', MouseBack: 'Кнопка мыши «Назад»',
  MouseForward: 'Кнопка мыши «Вперёд»', ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
  Backquote: '`', Minus: '−', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\',
  Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/',
};

export function bindingLabel({ modifiers, key }: VolumeGestureBinding) {
  const names = ['Ctrl', 'Shift', 'Alt', navigator.platform.includes('Mac') ? 'Cmd' : 'Super']
    .filter((_, index) => modifiers & (1 << index));
  if (key) names.push(labels[key] ?? key.replace(/^(Key|Digit)/, ''));
  return names.join(' + ');
}

export function isVolumeGestureKey(code: string) {
  return /^(Key[A-Z]|Digit[0-9]|F(?:[1-9]|1[0-2])|Arrow(?:Up|Down|Left|Right)|Space|Enter|Tab|Backspace|Delete|Home|End|PageUp|PageDown|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Comma|Period|Slash)$/.test(code);
}
