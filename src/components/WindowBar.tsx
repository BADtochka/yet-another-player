import { getCurrentWindow } from '@tauri-apps/api/window';

import { WindowClose } from './Icons/WindowClose';
import { WindowMaximize } from './Icons/WindowMaximize';
import { WindowMinimize } from './Icons/WindowMinimize';

export const WindowBar = () => {
  const appWindow = getCurrentWindow();
  const onWindowToggleMaximize = () => {
    void appWindow.toggleMaximize();
  };

  const onWindowControlsPointerDown = (event: PointerEvent) => {
    event.stopPropagation();
  };

  const onWindowControlsDoubleClick = (event: MouseEvent) => {
    event.stopPropagation();
  };

  const onWindowMinimize = () => {
    void appWindow.minimize();
  };

  const onWindowClose = () => {
    void appWindow.close();
  };

  return (
    <header
      className='sticky w-full top-0 flex h-12 select-none items-center px-2 font-onest bg-[#061232] z-100'
      data-tauri-drag-region
    >
      <div className='pointer-events-none absolute inset-0 flex items-center justify-center'>
        <span className='text-base font-semibold text-white'>YAP</span>
      </div>
      <div
        className='ml-auto flex items-center gap-2'
        onPointerDown={onWindowControlsPointerDown}
        onDblClick={onWindowControlsDoubleClick}
      >
        <button
          type='button'
          aria-label='Minimize'
          className='flex flex-col items-center justify-center gap-2.5 rounded-[5px] border-0 bg-transparent size-9 hover:bg-[#030d28] cursor-pointer'
          onClick={onWindowMinimize}
        >
          <WindowMinimize className='text-[#d9d9d9]' />
        </button>
        <button
          type='button'
          aria-label='Maximize'
          className='flex flex-row items-center justify-center gap-2.5 rounded-[5px] border-0 bg-transparent size-9 hover:bg-[#030d28] cursor-pointer'
          onClick={onWindowToggleMaximize}
        >
          <WindowMaximize className='text-[#d9d9d9]' />
        </button>
        <button
          type='button'
          aria-label='Close'
          className='flex flex-row items-center justify-center gap-2.5 rounded-[5px] border-0 bg-transparent size-9 hover:bg-[#da3e3e] cursor-pointer'
          onClick={onWindowClose}
        >
          <WindowClose className='text-[#d9d9d9]' />
        </button>
      </div>
    </header>
  );
};
