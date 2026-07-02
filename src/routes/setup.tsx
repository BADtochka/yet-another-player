import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { open } from '@tauri-apps/plugin-dialog';
import { useState } from 'preact/hooks';
import { useSettingsStore } from '@/stores';
import { type TrackScanProgress } from '@/utils/getTracks';
import { parseMusic } from '@/utils/parseMusic';

export const Route = createFileRoute('/setup')({
  component: RouteComponent,
});

function RouteComponent() {
  const addMusicFolder = useSettingsStore((state) => state.addMusicFolder);
  const [musicLoaded, setMusicLoaded] = useState(false);
  const [musicPath, setMusicPath] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [scanProgress, setScanProgress] = useState<TrackScanProgress | null>(null);
  const navigate = useNavigate();

  const selectMusicFolder = async () => {
    setError(null);
    setScanProgress(null);
    setIsLoading(true);

    try {
      const selectedPath = await open({
        title: 'Выберите папку с музыкой',
        directory: true,
        recursive: true,
        multiple: false,
      });

      if (typeof selectedPath !== 'string') {
        return;
      }

      setMusicPath(selectedPath);
      addMusicFolder(selectedPath);
      await parseMusic({ onProgress: setScanProgress });
      setMusicLoaded(true);
      await navigate({ to: '/tracks' });
    } catch (selectError) {
      setError(selectError instanceof Error ? selectError.message : String(selectError));
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className='flex flex-col w-full justify-center items-center gap-4 text-white'>
      <h1>Добро пожаловать</h1>
      <button
        type='button'
        className='rounded-xl bg-white px-4 py-2 font-semibold text-[#01040c] disabled:opacity-60'
        disabled={isLoading}
        onClick={selectMusicFolder}
      >
        {isLoading ? 'Сканирую музыку...' : 'Выбрать папку с музыкой'}
      </button>
      {scanProgress && (
        <p className='text-sm font-medium text-white/70'>
          {scanProgress.current} / {scanProgress.total ?? '...'}
        </p>
      )}
      {musicPath && <p className='max-w-xl text-center text-sm text-white/70'>{musicPath}</p>}
      {musicLoaded && <p className='text-sm text-[#73d13d]'>Музыка загружена</p>}
      {error && <p className='max-w-xl text-center text-sm text-[#da3e3e]'>{error}</p>}
    </div>
  );
}
