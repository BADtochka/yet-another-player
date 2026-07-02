import { useEffect, useMemo, useState } from 'preact/hooks';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { SettingsTextAction } from '@/components/Settings/SettingsTextAction';
import {
  type ExternalTool,
  type ExternalToolInstallProgress,
  type ExternalToolStatus,
  getExternalToolStatus,
  installExternalTool,
} from '@/utils/externalTools';

const TOOLS: Array<{ tool: ExternalTool; title: string; description: string }> = [
  {
    tool: 'yt-dlp',
    title: 'yt-dlp',
    description: 'CLI-инструмент для поиска и скачивания музыки из внешних источников',
  },
  {
    tool: 'ffmpeg',
    title: 'ffmpeg',
    description: 'CLI-инструмент для конвертации аудио, метаданных и расширенной обработки загрузок',
  },
];

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
  const [progress, setProgress] = useState<ExternalToolInstallProgress | null>(null);
  const [isInstalling, setIsInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const progressText = useMemo(() => {
    if (!progress || progress.status === 'installed') {
      return null;
    }

    if (progress.total && progress.total > 0) {
      const percent = Math.round((progress.downloaded / progress.total) * 100);
      return `${percent}% (${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)})`;
    }

    return formatBytes(progress.downloaded);
  }, [progress]);

  useEffect(() => {
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
  }, [tool]);

  const onInstall = async () => {
    setError(null);
    setProgress(null);
    setIsInstalling(true);

    try {
      const nextStatus = await installExternalTool(tool, setProgress);
      setStatus(nextStatus);
    } catch (installError) {
      setError(installError instanceof Error ? installError.message : String(installError));
    } finally {
      setIsInstalling(false);
    }
  };

  return (
    <div className='flex flex-col gap-2'>
      <div className='flex items-center justify-between gap-5'>
        <div className='flex min-w-0 flex-col gap-1'>
          <p className='text-base font-medium text-[#d9d9d9]'>{title}</p>
          <p className='text-sm font-medium text-[#8d8d8d]'>{description}</p>
          {status?.installed && status.version && (
            <p className='truncate text-sm font-medium text-[#8d8d8d]'>Установлено: {status.version.split('\n')[0]}</p>
          )}
        </div>

        <div className='flex shrink-0 items-center gap-4'>
          {progressText && <span className='text-sm font-medium text-[#8d8d8d]'>{progressText}</span>}
          {progress?.status === 'installed' && !isInstalling && (
            <span className='text-sm font-medium text-[#8d8d8d]'>Готово</span>
          )}
          <SettingsTextAction disabled={isInstalling} onClick={onInstall}>
            {isInstalling ? 'Установка...' : status?.installed ? 'Переустановить' : 'Установить'}
          </SettingsTextAction>
        </div>
      </div>

      {error && <p className='text-sm text-[#da3e3e]'>{error}</p>}
    </div>
  );
};

export const ExternalToolsSettings = () => {
  return (
    <SettingsSection title='Внешние инструменты'>
      <div className='flex flex-col gap-5 p-2.5'>
        {TOOLS.map((tool) => (
          <ExternalToolRow key={tool.tool} {...tool} />
        ))}
      </div>
    </SettingsSection>
  );
};
