import { useState } from 'preact/hooks';
import { open } from '@tauri-apps/plugin-dialog';
import { MediaFolderRow } from '@/components/Settings/MediaFolderRow';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { Button } from '@/components/Button';
import { Progress } from '@/components/Progress';
import { useSettingsStore } from '@/stores';
import { cancelMusicScan, createScanId, parseMusic, type TrackScanProgress } from '@/utils/parseMusic';

export const MediaFoldersSettings = () => {
  const musicFolders = useSettingsStore((state) => state.musicFolders);
  const setMusicFolders = useSettingsStore((state) => state.setMusicFolders);
  const [isAdding, setIsAdding] = useState(false);
  const [scanId, setScanId] = useState<string>();
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

      await onRescan([...new Set([...musicFolders, selectedPath])]);
    } catch (addError) {
      setError(addError instanceof Error ? addError.message : String(addError));
    } finally {
      setIsAdding(false);
    }
  };

  const onRescan = async (folders = musicFolders) => {
    setError(null);
    setScanProgress(null);
    const id = createScanId();
    setScanId(id);

    try {
      const result = await parseMusic({ id, folders, onProgress: setScanProgress });
      if (!result.cancelled) {
        setMusicFolders(folders);
        if (result.unavailableFolders > 0) {
          setError('Некоторые папки недоступны. Их треки сохранены в медиатеке.');
        }
      }
    } catch (rescanError) {
      setError(rescanError instanceof Error ? rescanError.message : String(rescanError));
    } finally {
      setScanId(undefined);
      setScanProgress(null);
    }
  };

  return (
    <SettingsSection title='Медиатека' description='Папки, в которых хранится и сканируется музыка'
      actions={musicFolders.length > 0 && !scanId && (
        <>
          <Button size='icon' icon='Folder' disabled={isAdding} aria-label='Добавить папку' title='Добавить папку' onClick={onAddFolder} />
          <Button size='icon' icon='Refresh' disabled={isAdding} aria-label='Пересканировать' title='Пересканировать'
            onClick={() => void onRescan()} />
        </>
      )}>
      {musicFolders.length > 0 && (
        <div className='settings-folder-list'>
          {musicFolders.map((path) => <MediaFolderRow key={path} path={path} disabled={isAdding || !!scanId}
            onRemove={() => void onRescan(musicFolders.filter((folder) => folder !== path))} />)}
        </div>
      )}
      {scanId ? (
        <div className='settings-scan' aria-live='polite'>
          <p>Сканирование медиатеки…</p>
          <Progress label='Сканирование медиатеки' value={scanProgress?.current} max={scanProgress?.total} counts />
          <Button variant='danger' icon='X' disabled={!scanProgress}
            onClick={() => void cancelMusicScan(scanId).catch((reason) => setError(String(reason)))}>Отменить сканирование</Button>
        </div>
      ) : musicFolders.length === 0 && (
        <Button variant='primary' icon='Folder' disabled={isAdding} onClick={onAddFolder}>
          {isAdding ? 'Выбираем папку…' : 'Добавить папку'}
        </Button>
      )}
      {error && <p className='kit-error' role='alert'>{error}</p>}
    </SettingsSection>
  );
};
