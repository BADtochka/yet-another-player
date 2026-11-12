import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { type AudioEnvironment, getAudioEnvironment } from '@/utils/audioEnvironment';
import { getConfigFolderPath, logError, openConfigFolder } from '@/utils/appLogs';

export function DiagnosticsSettings() {
  const [isOpening, setIsOpening] = useState(false);
  const [error, setError] = useState<string>();
  const [environment, setEnvironment] = useState<AudioEnvironment>();
  const [folder, setFolder] = useState<string>();

  useEffect(() => {
    void getAudioEnvironment().then(setEnvironment).catch(() => undefined);
    void getConfigFolderPath().then(setFolder).catch(() => undefined);
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
          {folder && <p className='settings-version truncate' title={folder}>{folder}</p>}
        </div>
        <Button variant='primary' icon='Open' disabled={isOpening} onClick={() => void open()}>
          {isOpening ? 'Открываем…' : 'Открыть папку'}
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
