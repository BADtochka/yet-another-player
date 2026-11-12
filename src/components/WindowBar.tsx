import { getCurrentWindow } from '@tauri-apps/api/window';
import { Button } from '@/components/Button';

export function WindowBar() {
  const window = getCurrentWindow();

  return (
    <header className='app-window-bar' data-tauri-drag-region>
      <span className='app-window-title'>YAP</span>
      <div className='app-window-actions'>
        <Button variant='ghost' size='icon' icon='Minus' aria-label='Свернуть окно' onClick={() => void window.minimize()} />
        <Button variant='ghost' size='icon' icon='Square' aria-label='Изменить размер окна' onClick={() => void window.toggleMaximize()} />
        <Button variant='ghost' size='icon' icon='X' className='app-window-close' aria-label='Закрыть окно' onClick={() => void window.close()} />
      </div>
    </header>
  );
}
