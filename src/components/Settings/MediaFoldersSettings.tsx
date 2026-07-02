import { useState } from 'preact/hooks';
import { open } from '@tauri-apps/plugin-dialog';
import { MediaFolderRow } from '@/components/Settings/MediaFolderRow';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { SettingsTextAction } from '@/components/Settings/SettingsTextAction';
import { useSettingsStore } from '@/stores';
import { type TrackScanProgress } from '@/utils/getTracks';
import { parseMusic } from '@/utils/parseMusic';

export const MediaFoldersSettings = () => {
  const musicFolders = useSettingsStore((state) => state.musicFolders);
  const addMusicFolder = useSettingsStore((state) => state.addMusicFolder);
  const removeMusicFolder = useSettingsStore((state) => state.removeMusicFolder);
  const [isAdding, setIsAdding] = useState(false);
  const [isRescanning, setIsRescanning] = useState(false);
  const [scanProgress, setScanProgress] = useState<TrackScanProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const onAddFolder = async () => {
    setError(null);
    setIsAdding(true);

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

      addMusicFolder(selectedPath);
    } catch (addError) {
      setError(addError instanceof Error ? addError.message : String(addError));
    } finally {
      setIsAdding(false);
    }
  };

  const onRescan = async () => {
    if (musicFolders.length === 0) {
      return;
    }

    setError(null);
    setScanProgress(null);
    setIsRescanning(true);

    try {
      await parseMusic({ onProgress: setScanProgress });
    } catch (rescanError) {
      setError(rescanError instanceof Error ? rescanError.message : String(rescanError));
    } finally {
      setIsRescanning(false);
    }
  };

  return (
    <SettingsSection title='Медиатека' description='Выберите папку с вашей музыкой'>
      <div className='flex flex-col gap-4 p-2.5'>
        <SettingsTextAction disabled={isAdding} onClick={onAddFolder}>
          {isAdding ? 'Выбор папки...' : 'Добавить новую папку'}
        </SettingsTextAction>

        {musicFolders.length > 0 && (
          <div className='flex flex-col gap-2.5 self-stretch'>
            {musicFolders.map((path) => (
              <MediaFolderRow key={path} path={path} onRemove={() => removeMusicFolder(path)} />
            ))}
          </div>
        )}

        {musicFolders.length > 0 && (
          <SettingsTextAction disabled={isRescanning} onClick={onRescan}>
            {isRescanning ? 'Обновление...' : 'Обновить'}
          </SettingsTextAction>
        )}

        {scanProgress && (
          <p className='text-sm font-medium text-[#8d8d8d]'>
            {scanProgress.current} / {scanProgress.total ?? '...'}
          </p>
        )}

        {error && <p className='text-sm text-[#da3e3e]'>{error}</p>}
      </div>
    </SettingsSection>
  );
};
