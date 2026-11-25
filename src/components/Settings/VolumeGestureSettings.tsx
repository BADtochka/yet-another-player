import { useEffect, useLayoutEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { BetaBadge } from '@/components/Settings/BetaBadge';
import { Switch } from '@/components/Switch';
import { useSettingsStore } from '@/stores/settingsStore';
import { activeVolumeGesture, bindingLabel, canRetryVolumeGestureAccess, configureVolumeGesture, isVolumeGestureKey, modifierMask, mouseBinding, useVolumeGesture } from '@/stores/volumeGestureStore';
import type { VolumeGestureBinding } from '@/types/Settings';

const statusText = {
  off: '',
  global: 'Прокрутка с назначенной привязкой меняет громкость YAP и перехватывается в других окнах.',
  x11: 'Работает поверх других приложений. В X11 исходная прокрутка также передаётся активному окну.',
  wayland: 'В Wayland жест работает внутри YAP. Глобальный жест и внешний индикатор доступны в X11.',
  local: 'Системный обработчик недоступен. Жест работает внутри окна YAP.',
  permissionRequired: 'В разделе «Конфиденциальность и безопасность» откройте «Управление устройством и доступ к данным» или «Универсальный доступ», разрешите доступ для YAP и вернитесь в плеер. Пока жест работает внутри YAP.',
  macInterceptionUnavailable: 'Для YAP включён доступ, но перехват прокрутки недоступен. Проверьте разрешение в системных настройках и перезапустите YAP. Пока жест работает внутри плеера.',
};

export function VolumeGestureSettings() {
  const title = 'Громкость по прокрутке';
  const binding = useSettingsStore((state) => state.volumeGesture);
  const enabled = useSettingsStore((state) => Boolean(activeVolumeGesture(state)));
  const saveBinding = useSettingsStore((state) => state.setVolumeGesture);
  const setEnabled = useSettingsStore((state) => state.setVolumeGestureEnabled);
  const { status, pending, error, recording } = useVolumeGesture();
  const [open, setOpen] = useState(false);
  const [recordError, setRecordError] = useState('');
  const [preview, setPreview] = useState('');

  const dismiss = () => {
    useVolumeGesture.setState({ recording: false });
    setRecordError('');
    setOpen(false);
  };

  useLayoutEffect(() => {
    if (!recording) return;
    let modifiers = 0;
    let done = false;
    const stop = () => {
      done = true;
      useVolumeGesture.setState({ recording: false });
    };
    const finish = (selection: VolumeGestureBinding) => {
      stop();
      setRecordError('');
      saveBinding(selection);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (done) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.repeat) return;
      if (event.key === 'Escape') {
        stop();
        return;
      }
      if (event.getModifierState('AltGraph')) {
        setRecordError('Используйте Ctrl, Shift, Alt или системную клавишу.');
        return;
      }
      if (/^(Control|Shift|Alt|Meta)$/.test(event.key)) {
        modifiers |= modifierMask(event);
        setPreview(bindingLabel({ modifiers, key: null }));
      } else if (isVolumeGestureKey(event.code)) {
        finish({ modifiers: modifierMask(event), key: event.code });
      } else {
        modifiers = 0;
        setPreview('');
        setRecordError('Эта клавиша не поддерживается.');
      }
    };
    const handleKeyUp = (event: KeyboardEvent) => {
      if (!done && modifiers && /^(Control|Shift|Alt|Meta)$/.test(event.key)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        finish({ modifiers, key: null });
      }
    };
    const handlePointer = (event: PointerEvent) => {
      if (done) return;
      const key = mouseBinding(event.button);
      if (key) {
        event.preventDefault();
        event.stopImmediatePropagation();
        finish({ modifiers: modifierMask(event), key });
      } else {
        stop();
      }
    };
    window.addEventListener('keydown', handleKeyDown, true);
    window.addEventListener('keyup', handleKeyUp, true);
    window.addEventListener('pointerdown', handlePointer, true);
    window.addEventListener('blur', stop);
    return () => {
      window.removeEventListener('keydown', handleKeyDown, true);
      window.removeEventListener('keyup', handleKeyUp, true);
      window.removeEventListener('pointerdown', handlePointer, true);
      window.removeEventListener('blur', stop);
    };
  }, [recording, saveBinding]);

  useEffect(() => () => {
    useVolumeGesture.setState({ recording: false });
  }, []);

  return <div className='settings-switch'>
    <div className='settings-row'>
      <p>{title}<BetaBadge /></p>
      <div className='settings-row-actions'>
        <Button size='icon' icon='Settings' aria-label='Настройки громкости по прокрутке'
          title='Настройки громкости по прокрутке' onClick={() => setOpen(true)} />
        <Switch label={title} checked={enabled} onChange={(checked) => {
          if (checked && !binding) setOpen(true);
          else setEnabled(checked);
        }} />
      </div>
    </div>
    <Dialog open={open} title={title} onDismiss={dismiss}
      description='Удерживайте назначенную клавишу, сочетание или кнопку мыши и прокручивайте вверх или вниз.'>
      {binding && <p>Привязка <strong>{bindingLabel(binding)}</strong></p>}
      {recording && <p role='status'>{preview || 'Нажмите клавишу, сочетание, среднюю или боковую кнопку мыши. Escape отменяет ввод.'}</p>}
      {(error || recordError) && <p className='kit-error' role='alert'>{recordError || error}</p>}
      {pending && <LoadingIndicator label='Настройка жеста громкости' />}
      {statusText[status] && <p role='status'>{statusText[status]}</p>}
      {enabled && canRetryVolumeGestureAccess(status) && <div className='kit-dialog-actions'>
        <Button size='small' disabled={pending} onClick={() => configureVolumeGesture(binding, true)}>
          Открыть системные настройки
        </Button>
        <Button size='small' disabled={pending} onClick={() => configureVolumeGesture(binding)}>Проверить доступ</Button>
      </div>}
      <div className='kit-dialog-actions'>
        {binding && <Button variant='ghost' onClick={() => saveBinding(null)}>Удалить привязку</Button>}
        <Button variant='primary' data-initial-focus aria-pressed={recording} onClick={() => {
          setPreview('');
          setRecordError('');
          useVolumeGesture.setState({ recording: true });
        }}>Назначить</Button>
      </div>
    </Dialog>
  </div>;
}
