import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { open } from '@tauri-apps/plugin-dialog';
import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Icon, type IconName } from '@/components/Icons/Icon';
import { Progress } from '@/components/Progress';
import { useSettingsStore } from '@/stores';
import { registerMusicRoots } from '@/utils/musicFolders';
import { cancelMusicScan, createScanId, parseMusic, type TrackScanProgress } from '@/utils/parseMusic';

export const Route = createFileRoute('/setup')({
  component: Setup,
});

const features: Array<{ icon: IconName; title: string; description: string }> = [
  { icon: 'Folder', title: 'Локально', description: 'Библиотека без трекеров и метрик' },
  { icon: 'Color', title: 'Оформление', description: 'Список и сетка для вашей медиатеки' },
  { icon: 'Playlist', title: 'Плейлисты', description: 'Плейлисты и подборки для удобной сортировки' },
  { icon: 'Play', title: 'Управление', description: 'Привычные элементы управления и медиаклавиши' },
  { icon: 'Settings', title: 'Настройки', description: 'Настройки медиатеки, инструментов и громкости' },
  { icon: 'Lyrics', title: 'Текст треков', description: 'Синхронизированный текст из локальных файлов' },
  { icon: 'Download', title: 'Внешние инструменты', description: 'Пополнение библиотеки открытыми инструментами' },
  { icon: 'Queue', title: 'Очередь', description: 'Порядок воспроизведения всегда под рукой' },
];

function Setup() {
  const musicFolders = useSettingsStore((state) => state.musicFolders);
  const addMusicFolder = useSettingsStore((state) => state.addMusicFolder);
  const [selecting, setSelecting] = useState(false);
  const [scanId, setScanId] = useState<string>();
  const [progress, setProgress] = useState<TrackScanProgress>();
  const [trackCount, setTrackCount] = useState<number>();
  const [error, setError] = useState<string>();
  const navigate = useNavigate();

  useEffect(() => {
    if (trackCount === undefined) return;
    const timeout = window.setTimeout(() => void navigate({ to: '/tracks' }), 800);
    return () => window.clearTimeout(timeout);
  }, [trackCount, navigate]);

  const selectMusicFolder = async () => {
    setError(undefined);
    setSelecting(true);
    try {
      const path = await open({
        title: 'Выберите папку с музыкой',
        directory: true,
        recursive: true,
        multiple: false,
      });
      if (typeof path !== 'string') return;

      const id = createScanId();
      setScanId(id);
      const result = await parseMusic({
        id,
        folders: [...new Set([...musicFolders, path])],
        onProgress: setProgress,
      });

      if (result.cancelled) return;
      if (result.catalog.trackCount === 0) {
        await registerMusicRoots(musicFolders);
        setError(result.unavailableFolders > 0 ? 'Не удалось прочитать папку.' : 'В этой папке нет аудиофайлов.');
        return;
      }

      addMusicFolder(path);
      setTrackCount(result.catalog.trackCount);
    } catch (error) {
      await registerMusicRoots(musicFolders).catch(() => undefined);
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSelecting(false);
      setScanId(undefined);
      setProgress(undefined);
    }
  };

  if (trackCount !== undefined) {
    return <section className='setup-screen setup-done' role='status'>
      <img className='setup-logo' src='/logo.svg' alt='YAP' />
      <span className='sr-only'>Медиатека готова. Треков найдено — {trackCount}</span>
    </section>;
  }

  return (
    <section className='setup-screen'>
      <img className='setup-logo' src='/logo.svg' alt='YAP' />
      <h1>Слушай музыку. <span>Свободно.</span></h1>
      <ul className='setup-features'>
        {features.map(({ icon, title, description }) => (
          <li key={title}>
            <h2><Icon name={icon} size={32} />{title}</h2>
            <p>{description}</p>
          </li>
        ))}
      </ul>
      <div className='setup-panel' aria-live='polite'>
        {scanId ? (
          <>
            <p>Поиск музыки в выбранной папке…</p>
            <Progress label='Сканирование музыки' max={progress?.total} value={progress?.current} counts />
            <Button variant='danger' icon='X' disabled={!progress}
              onClick={() => void cancelMusicScan(scanId).catch((reason) => setError(String(reason)))}>Отмена</Button>
          </>
        ) : (
          <>
            <p>Выберите папку с вашей музыкальной библиотекой или укажите место, где хотите собрать новую.</p>
            <Button variant='primary' icon='Folder' disabled={selecting} onClick={() => void selectMusicFolder()}>
              {selecting ? 'Выбираем папку…' : 'Выбрать папку'}
            </Button>
          </>
        )}
        {error && <p role='alert' className='kit-error'>{error}</p>}
      </div>
    </section>
  );
}
