import { Channel, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useState } from 'preact/hooks';

import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { Icon } from '@/components/Icons/Icon';
import { Input } from '@/components/Input';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Progress } from '@/components/Progress';
import { BetaBadge } from '@/components/Settings/BetaBadge';
import { Switch } from '@/components/Switch';
import { LibrarySyncSettings } from '@/components/Settings/LibrarySyncSettings';
import { setSyncServerAccess, useSyncServerAccess } from '@/stores/syncServerAccessStore';

const SOURCE_URL = 'https://github.com/BADtochka/yap-sync-server';

type ServerStatus = {
  installed: boolean;
  address: string | null;
  asset: string | null;
  enabled: boolean;
  needsUpdate: boolean;
  error: string | null;
};

type InstallProgress = { status: string; downloaded: number; total: number | null };

export function SyncServerSettings() {
  const [status, setStatus] = useState<ServerStatus>();
  const [busy, setBusy] = useState<'toggle' | 'install' | 'pick'>();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { devices, action: accessBusy, accessError, loadError: deviceLoadError } = useSyncServerAccess();
  const [error, setError] = useState('');
  const [installError, setInstallError] = useState('');
  const [installOpen, setInstallOpen] = useState(false);
  const [source, setSource] = useState('');
  const [local, setLocal] = useState(false);
  const [checksum, setChecksum] = useState('');
  const [progress, setProgress] = useState<InstallProgress>();

  const loadStatus = async () => {
    setError('');
    try {
      const result = await invoke<ServerStatus>('sync_server_status');
      setStatus(result);
    } catch (error) {
      setError(String(error));
    }
  };

  useEffect(() => {
    if (busy) return;
    let active = true;
    let pending = false;
    const update = async () => {
      if (pending) return;
      pending = true;
      try {
        const result = await invoke<ServerStatus>('sync_server_status');
        if (active) setStatus(result);
      } catch (error) {
        if (active) setError(String(error));
      } finally {
        pending = false;
      }
    };
    void update();
    const poll = window.setInterval(update, 2_000);
    return () => { active = false; window.clearInterval(poll); };
  }, [busy]);

  const pickBinary = async () => {
    setBusy('pick');
    setInstallError('');
    try {
      const file = await open({ directory: false, multiple: false, title: 'Файл сервера YAP Sync' });
      if (typeof file === 'string') {
        setSource(file);
        setLocal(true);
      }
    } catch {
      setInstallError('Не удалось выбрать файл сервера.');
    } finally {
      setBusy(undefined);
    }
  };

  const install = async (manual: boolean) => {
    if (busy) return;
    setBusy('install');
    setError('');
    setInstallError('');
    setProgress(undefined);
    const events = new Channel<InstallProgress>(setProgress);
    try {
      const installed = await invoke<ServerStatus>('install_sync_server', {
        source: manual ? { location: source.trim(), checksum: checksum.trim(), local } : null,
        progress: events,
      });
      setStatus(installed);
      setInstallOpen(false);
    } catch (error) {
      if (manual) setInstallError(String(error));
      else setError(String(error));
    } finally {
      setBusy(undefined);
    }
  };

  const toggle = async (enabled: boolean) => {
    if (busy) return;
    setBusy('toggle');
    setError('');
    try {
      setStatus(await invoke<ServerStatus>('sync_server_set_enabled', { enabled }));
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(undefined);
    }
  };

  const downloading = progress?.status === 'downloading';
  const installProgress = <div className='settings-install-progress' aria-live='polite'>
    <p>{downloading ? 'Загрузка сервера' : 'Проверка файла сервера'}</p>
    <Progress label='Установка сервера' value={downloading ? progress?.downloaded : undefined}
      max={downloading ? progress?.total : undefined} />
  </div>;

  return <section className='settings-tool-category' aria-labelledby='tools-sync-server'>
    <h3 id='tools-sync-server'>Синхронизация</h3>
    <div className='settings-row'>
      <div className='settings-row-info'>
        <h4>
          <button type='button' className='settings-source' aria-label='Исходный код сервера YAP Sync'
            onClick={() => void openUrl(SOURCE_URL).catch(() => setError('Не удалось открыть исходники YAP Sync.'))}>
            Сервер YAP Sync <Icon name='Open' size={16} />
          </button>
          <BetaBadge />
        </h4>
        <p>Синхронизирует треки, плейлисты, метаданные и счётчики прослушиваний между устройствами.</p>
      </div>
      <div className='settings-row-actions'>
        {busy === 'install' && !installOpen && installProgress}
        {status && (!status.installed || status.needsUpdate) && <>
          <Button variant='ghost' size='small' disabled={!!busy} onClick={() => {
            setInstallError('');
            setInstallOpen(true);
          }}>Другой источник</Button>
          <Button variant='primary' icon='Install' pending={busy === 'install' && !installOpen}
            disabled={!!busy || !status.asset} onClick={() => void install(false)}>
            {status.needsUpdate ? 'Обновить' : 'Установить'}
          </Button>
        </>}
        {!status && !error && <LoadingIndicator label='Проверка сервера YAP Sync' />}
        {!status && error && <Button onClick={loadStatus}>Повторить</Button>}
        <Button size='icon' icon='Settings' aria-label='Настройки YAP Sync' title='Настройки YAP Sync'
          onClick={() => setSettingsOpen(true)} />
        {busy === 'toggle' && <LoadingIndicator label='Изменение состояния сервера YAP Sync' />}
        {status?.installed && !status.needsUpdate && <Switch label='Использовать сервер YAP Sync'
          checked={status.enabled} disabled={!!busy} onChange={toggle} />}
      </div>
    </div>
    {(error || status?.error) && <p className='kit-error' role='alert'>{error || status?.error}</p>}
    {settingsOpen && <Dialog open title='Настройки YAP Sync' className='sync-settings-dialog'
      onDismiss={() => setSettingsOpen(false)}>
      <LibrarySyncSettings />
      <section className='sync-modal-section' aria-labelledby='sync-host-title'>
        <h3 id='sync-host-title'>Сервер и список доступа</h3>
        {!status?.address && <p>{status?.needsUpdate ? 'Обновите сервер, чтобы включить список доступа.'
          : status?.error || 'Включите установленный сервер в разделе «Инструменты».'}</p>}
        {status?.address && <>
          <p>Сервер работает по адресу <strong>{status.address}</strong>.</p>
          {(accessError || deviceLoadError) && <p className='kit-error' role='alert'>{accessError || deviceLoadError}</p>}
          {!devices && !deviceLoadError && <LoadingIndicator label='Загрузка списка устройств' />}
          {devices && <ul className='sync-device-list' aria-label='Список доступа устройств'>
            {devices.map((device) => <li key={device.id}>
              <div className='settings-row-info'>
                <strong>{device.name}{device.local ? ' · Этот компьютер' : ''}</strong>
                <small>
                  {device.status === 'pending' ? 'Ожидает подтверждения'
                    : device.status === 'allowed' ? 'Разрешено' : 'Доступ закрыт'}
                </small>
              </div>
              {!device.local && <div className='sync-actions'>
                {device.status !== 'allowed' && <Button variant='primary' size='small'
                  pending={accessBusy?.id === device.id && accessBusy.allowed} disabled={!!accessBusy} onClick={() => void setSyncServerAccess(device.id, true)}>Разрешить</Button>}
                {device.status !== 'blocked' && <Button size='small' disabled={!!accessBusy}
                  pending={accessBusy?.id === device.id && !accessBusy.allowed}
                  onClick={() => void setSyncServerAccess(device.id, false)}>Закрыть доступ</Button>}
              </div>}
            </li>)}
          </ul>}
          {devices?.every((device) => device.local) && <p>Другие устройства появятся после запроса на подключение.</p>}
        </>}
      </section>
    </Dialog>}
    {installOpen && <Dialog open title='Установить сервер YAP Sync' busy={!!busy}
      description={status?.asset ? `Файл для этой платформы — ${status.asset}.` : undefined}
      onDismiss={() => setInstallOpen(false)}>
      <div className='sync-server-form'>
        <label>
          <span>{local ? 'Локальный файл сервера' : 'Ссылка на файл из GitHub Releases'}</span>
          <Input value={source} disabled={!!busy} onInput={(event) => {
            setLocal(false);
            setSource(event.currentTarget.value);
          }} />
        </label>
        <Button disabled={!!busy} pending={busy === 'pick'} onClick={pickBinary}>Выбрать локальный файл</Button>
        <label>
          <span>Контрольная сумма SHA-256</span>
          <Input value={checksum} maxLength={64} disabled={!!busy}
            onInput={(event) => setChecksum(event.currentTarget.value.trim())} />
        </label>
        {busy === 'install' && installProgress}
        {installError && <p className='kit-error' role='alert'>{installError}</p>}
      </div>
      <div className='kit-dialog-actions'>
        <Button disabled={!!busy} onClick={() => setInstallOpen(false)}>Закрыть</Button>
        <Button variant='primary' pending={busy === 'install'} disabled={!!busy || !source.trim() || !/^[0-9a-f]{64}$/i.test(checksum)}
          onClick={() => void install(true)}>Установить</Button>
      </div>
    </Dialog>}
  </section>;
}
