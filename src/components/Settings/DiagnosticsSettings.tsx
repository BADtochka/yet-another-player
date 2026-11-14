import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Icon } from '@/components/Icons/Icon';
import { type AudioEnvironment, getAudioEnvironment } from '@/utils/audioEnvironment';
import { getConfigFolderPath, logError, openConfigFolder } from '@/utils/appLogs';

export function DiagnosticsSettings() {
  const [isOpening, setIsOpening] = useState(false);
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [environment, setEnvironment] = useState<AudioEnvironment>();
  const [folder, setFolder] = useState<string>();

  useEffect(() => {
    let active = true;
    void Promise.allSettled([getAudioEnvironment(), getConfigFolderPath()]).then(([audio, config]) => {
      if (!active) return;
      if (audio.status === 'fulfilled') setEnvironment(audio.value);
      if (config.status === 'fulfilled') setFolder(config.value);
      setLoading(false);
    });
    return () => { active = false; };
  }, []);

  const open = async () => {
    setError(undefined);
    setIsOpening(true);
    try {
      await openConfigFolder();
    } catch (reason) {
      setError(String(reason));
      logError('Failed to open config folder', reason);
    } finally {
      setIsOpening(false);
    }
  };

  return (
    <>
      <div className='settings-row'>
        <div className='settings-row-info'>
          <h3><Icon name='Folder' size={20} />Логи и конфигурация</h3>
          {loading && <LoadingIndicator label='Загрузка диагностики' />}
          {folder && <p className='settings-version truncate' title={folder}>{folder}</p>}
        </div>
        <Button variant='primary' icon='Open' pending={isOpening} onClick={() => void open()}>
          Открыть папку
        </Button>
      </div>
      {environment && (
        <details className='settings-diagnostics'>
          <summary>Диагностика звука</summary>
          <p>Платформа — {environment.platform}{environment.isWsl ? ' (WSL)' : ''}</p>
          {environment.pulseServer && <p>PulseAudio — {environment.pulseServer}</p>}
          {environment.pulseLatencyMs && <p>Буфер — {environment.pulseLatencyMs} мс</p>}
          {environment.notes.length > 0 && <ul>{environment.notes.map((note) => <li key={note}>{note}</li>)}</ul>}
        </details>
      )}
      {error && <p className='kit-error' role='alert'>{error}</p>}
    </>
  );
}
