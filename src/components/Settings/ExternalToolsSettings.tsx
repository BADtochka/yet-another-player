import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useMemo, useState } from 'preact/hooks';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { Dialog } from '@/components/Dialog';
import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { Progress } from '@/components/Progress';
import { YtDlpChecking } from '@/components/YtDlpFeedback';
import {
  type ExternalTool,
  type ExternalToolStatus,
  cancelExternalToolInstall,
  getExternalToolStatus,
  installExternalTool,
  removeExternalTool,
  useToolInstalls,
} from '@/utils/externalTools';
import { useYtDlpStatus } from '@/utils/ytDlp';

const TOOLS: Array<{ tool: ExternalTool; title: string; description: string }> = [
  {
    tool: 'yt-dlp',
    title: 'yt-dlp',
    description: 'Поиск и скачивание музыки из внешних источников',
  },
  {
    tool: 'ffmpeg',
    title: 'ffmpeg',
    description: 'Конвертация аудио, метаданных и обработка скачанной музыки',
  },
];

const toolPages = { 'yt-dlp': 'https://github.com/yt-dlp/yt-dlp', ffmpeg: 'https://ffmpeg.org/' };
const ytDlpDate = (version: string | null | undefined) => version?.match(/^\d{4}\.\d{2}\.\d{2}(?:\.\d+)?(?=$|\s)/)?.[0];

let latestVersion: Promise<string> | undefined;
const checkLatestVersion = () => latestVersion ??= invoke<string>('latest_yt_dlp_version').catch((error) => {
  latestVersion = undefined;
  throw error;
});

const formatBytes = (bytes: number) => {
  if (bytes < 1024 * 1024) {
    return `${Math.round(bytes / 1024)} КБ`;
  }

  return `${(bytes / 1024 / 1024).toFixed(1)} МБ`;
};

type ExternalToolRowProps = {
  description: string;
  title: string;
  tool: ExternalTool;
};

const ExternalToolRow = ({ description, title, tool }: ExternalToolRowProps) => {
  const [status, setStatus] = useState<ExternalToolStatus | null>(null);
  const [latest, setLatest] = useState<string>();
  const job = useToolInstalls((state) => state[tool]);
  const progress = job?.progress;
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isChecking = useYtDlpStatus((state) => tool === 'yt-dlp' && state.checking);

  const progressText = useMemo(() => {
    if (progress?.status === 'extracting') return 'Распаковка…';
    if (progress?.status === 'checking' && tool === 'ffmpeg') return 'Проверка запуска…';
    if (progress?.status === 'waitingForDownload') return 'Ждём окончания работы инструмента…';
    if (progress?.status === 'publishing') return 'Завершаем установку…';
    if (progress?.status !== 'downloading') {
      return null;
    }

    if (progress.total && progress.total > 0) {
      const percent = Math.round((progress.downloaded / progress.total) * 100);
      return `${percent}% (${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)})`;
    }

    return formatBytes(progress.downloaded);
  }, [progress, tool]);

  useEffect(() => {
    if (job) return;
    let isMounted = true;

    getExternalToolStatus(tool)
      .then((nextStatus) => {
        if (isMounted) {
          setStatus(nextStatus);
        }
      })
      .catch((statusError) => {
        if (isMounted) {
          setError(statusError instanceof Error ? statusError.message : String(statusError));
        }
      });

    return () => {
      isMounted = false;
    };
  }, [tool, job?.id]);

  useEffect(() => {
    if (tool !== 'yt-dlp' || !ytDlpDate(status?.version)) return;
    let active = true;
    void checkLatestVersion().then((version) => { if (active) setLatest(version); }).catch(() => undefined);
    return () => { active = false; };
  }, [tool, status?.version]);

  const onInstall = async () => {
    setError(null);
    try {
      setStatus(await installExternalTool(tool));
    } catch (reason) {
      if (reason !== 'installCancelled') setError(String(reason));
    }
  };

  const retryCheck = async () => {
    setError(null);
    try {
      setStatus(await getExternalToolStatus(tool, true));
    } catch (reason) {
      setError(String(reason));
    }
  };

  const onRemove = async () => {
    setRemoving(true);
    setError(null);
    try {
      setStatus(await removeExternalTool(tool));
      setConfirmRemove(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setRemoving(false);
    }
  };

  const busy = !!job || isChecking || removing;
  const version = status?.version?.split('\n')[0];
  const currentDate = ytDlpDate(version);
  const newer = tool === 'yt-dlp' && currentDate && latest && latest.localeCompare(currentDate, 'en', { numeric: true }) > 0;
  return (
    <div className='settings-tool'>
      <div className='settings-row'>
        <div className='settings-row-info'>
          <h3>
            <button type='button' className='settings-source' aria-label={`Открыть сайт ${title}`}
              onClick={() => void openUrl(toolPages[tool]).catch((reason) => setError(String(reason)))}>
              {title} <Icon name='Open' size={16} />
            </button>
            {version && !job && <span className='settings-version'>
              {version}{newer ? ` → ${latest}` : ''}
            </span>}
          </h3>
          <p>{description}</p>
        </div>
        <div className='settings-row-actions'>
          {job ? (
            <>
              <div className='settings-install-progress' aria-live='polite'>
                <p>{job.cancelling ? 'Отменяем установку…' : isChecking ? 'Проверяем запуск yt-dlp…' : progressText ?? 'Установка инструмента'}</p>
                <Progress label='Установка инструмента' value={progress?.status === 'downloading' ? progress.downloaded : undefined}
                  max={progress?.status === 'downloading' ? progress.total : undefined} />
              </div>
              <Button size='icon' icon='X' aria-label={`Отменить установку ${title}`}
                disabled={job.cancelling || progress?.status === 'publishing' || progress?.status === 'installed' || isChecking}
                onClick={() => void cancelExternalToolInstall(tool).catch((reason) => setError(String(reason)))} />
            </>
          ) : (
            <>
              {isChecking && <YtDlpChecking />}
              {status?.launchError && !busy && <Button size='small' icon='Refresh' onClick={() => void retryCheck()}>Проверить снова</Button>}
              {status?.installed && !newer && !status.launchError ? (
                <>
                  <span className='settings-installed'><Icon name='Check' />Установлено</span>
                  <Button size='icon' icon='Refresh' disabled={busy} aria-label={`Переустановить ${title}`} onClick={() => void onInstall()} />
                </>
              ) : (
                <Button variant='primary' icon='Install' data-update={newer ? 'true' : undefined}
                  disabled={busy || (!status && !error)} onClick={() => void onInstall()}>
                  {newer ? 'Доступно обновление' : status?.installed ? 'Переустановить' : 'Установить'}
                </Button>
              )}
              {status?.installed && <Button size='icon' className='settings-folder-remove' icon='Trash' disabled={busy}
                aria-label={`Удалить ${title}`} onClick={() => { setError(null); setConfirmRemove(true); }} />}
            </>
          )}
        </div>
      </div>
      {!confirmRemove && (error || (!busy && status?.launchError)) && (
        <p className='kit-error' role='alert'>{error || status?.launchError}</p>
      )}
      <Dialog open={confirmRemove} title={`Удалить ${title}?`} busy={removing} onDismiss={() => setConfirmRemove(false)}
        description='Инструмент можно будет установить снова из настроек.'>
        {error && <p className='kit-error' role='alert'>{error}</p>}
        <div className='kit-dialog-actions'>
          <Button disabled={removing} onClick={() => setConfirmRemove(false)}>Отмена</Button>
          <Button variant='danger' icon='Trash' disabled={removing} onClick={() => void onRemove()}>Удалить</Button>
        </div>
      </Dialog>
    </div>
  );
};

export const ExternalToolsSettings = () => (
  <SettingsSection title='Внешние инструменты'
    description='Инструменты для декодирования, скачивания и получения метаданных треков'>
    <div className='settings-rows'>
      {TOOLS.map((tool) => <ExternalToolRow key={tool.tool} {...tool} />)}
    </div>
  </SettingsSection>
);
