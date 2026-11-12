import { getVersion } from '@tauri-apps/api/app';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { checkForAppUpdatesManually, installUpdate, useAvailableUpdate } from '@/utils/checkForUpdates';
import { toast } from '@/utils/toast';

export function UpdateSettings() {
  const [version, setVersion] = useState<string>();
  const [checking, setChecking] = useState(false);
  const { update, installing, pacmanCommand } = useAvailableUpdate();

  useEffect(() => {
    void getVersion().then(setVersion).catch(() => undefined);
  }, []);

  const check = async () => {
    if (update) { await installUpdate(update); return; }
    setChecking(true);
    try {
      await checkForAppUpdatesManually();
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className='settings-row'>
      <div className='settings-row-info'>
        <h3><button type='button' className='settings-source' aria-label='Открыть репозиторий YAP'
          onClick={() => void openUrl('https://github.com/BADtochka/yet-another-player')
            .catch((reason) => toast.error('Не удалось открыть репозиторий', reason))}>
          yap <Icon name='Github' size={20} />
        </button></h3>
        {version && <p className='settings-version'>v{version}{update && ` → v${update.version}`}</p>}
        {pacmanCommand && <code className='select-text break-all'>{pacmanCommand}</code>}
      </div>
      <Button variant='primary' icon={update ? 'Install' : 'Refresh'} data-update={update ? 'true' : undefined}
        disabled={checking || installing} onClick={() => void check()}>
        {installing ? 'Загружаем…' : checking ? 'Проверяем…' : update?.kind === 'arch' ? 'Скачать пакет' : update ? 'Доступно обновление' : 'Проверить обновление'}
      </Button>
    </div>
  );
}
