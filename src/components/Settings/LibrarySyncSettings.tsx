import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';

import { Button } from '@/components/Button';
import { Progress } from '@/components/Progress';
import { Select } from '@/components/Select';
import { Switch } from '@/components/Switch';
import { useSettingsStore } from '@/stores';
import { cancelLibrarySync, configureLibrarySync, loadLibrarySync, runLibrarySync, useLibrarySync } from '@/stores/librarySyncStore';

const phaseLabels = {
  preparing: 'Проверка подключения',
  indexing: 'Сверка локальных треков',
  sending: 'Отправка треков',
  receiving: 'Получение треков',
  applying: 'Обновление медиатеки и плейлистов',
  publishing: 'Публикация изменений',
};
const warningLabels: Record<string, string> = {
  peerManifest: 'Не удалось прочитать список данных устройства',
  localTrack: 'Локальный файл недоступен или изменился после сканирования медиатеки',
  trackMetadata: 'Метаданные трека не удалось подготовить',
  sendTrack: 'Не удалось отправить файл',
  receiveTrack: 'Не удалось получить файл или проверить его целостность',
  publish: 'Не удалось опубликовать изменения устройства',
};

const number = new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 1 });
const date = new Intl.DateTimeFormat('ru-RU', { dateStyle: 'medium', timeStyle: 'short' });

function formatBytes(bytes: number) {
  if (bytes >= 1024 ** 3) return `${number.format(bytes / 1024 ** 3)} ГБ`;
  if (bytes >= 1024 ** 2) return `${number.format(bytes / 1024 ** 2)} МБ`;
  if (bytes >= 1024) return `${number.format(bytes / 1024)} КБ`;
  return `${number.format(bytes)} Б`;
}

type Peer = { address: string; name: string; invite: string };

export function LibrarySyncSettings() {
  const { status, running, configuring, cancelling, cancelled, approvalDeadline, progress, error } = useLibrarySync();
  const awaitingApproval = Boolean(approvalDeadline);
  const folders = useSettingsStore((state) => state.musicFolders);
  const config = status?.config;
  const report = status?.report;
  const [folder, setFolder] = useState('');
  const [automatic, setAutomatic] = useState(false);
  const [peers, setPeers] = useState<Peer[]>();
  const [peer, setPeer] = useState<Peer>();
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState('');

  useEffect(() => {
    setFolder(config?.destination ?? '');
    setAutomatic(config?.automatic ?? false);
    setPeer(undefined);
  }, [config]);

  const findPeers = async () => {
    setScanning(true);
    setScanError('');
    try {
      setPeers(await invoke<Peer[]>('library_sync_discover'));
    } catch (reason) {
      setScanError(String(reason));
    } finally {
      setScanning(false);
    }
  };

  const invite = peer?.invite ?? config?.invite ?? null;
  const legacy = Boolean(config && !config.invite);
  const target = folder || folders[0] || '';
  const connecting = Boolean(peer && peer.invite !== config?.invite);
  const dirty = config?.destination !== target || connecting || config?.automatic !== automatic;
  const busy = running || configuring;
  const canCancel = running && progress?.phase !== 'applying' && progress?.phase !== 'publishing';
  const ready = folders.includes(target) && (Boolean(invite) || legacy);
  const save = async () => {
    if (await configureLibrarySync({ folder: target, automatic, invite }) && connecting) {
      void runLibrarySync();
    }
  };

  return <section className='sync-modal-section' aria-labelledby='sync-client-title'>
    <h3 id='sync-client-title'>Подключение</h3>
    {status ? <>
      <div className='sync-folders'>
        <div className='settings-row'>
          <div className='settings-row-info'>
            <h3>Папка медиатеки</h3>
            <p>YAP отправляет треки из этой папки и сохраняет полученные в её подпапку «YAP Sync».</p>
          </div>
          <Select label='Папка медиатеки для синхронизации' className='sync-destination' value={target}
            options={folders.map((path) => ({ value: path, label: path }))}
            onChange={setFolder} disabled={busy} placeholder='Выберите папку медиатеки' />
        </div>
        <div className='sync-server-form'>
          <div className='settings-row'>
            <div className='settings-row-info'>
              <h3>Сервер в локальной сети</h3>
              <p>{status.address ? `Выбранный сервер · ${status.address}`
                : legacy ? 'Сейчас настроен обмен через общую папку.'
                : 'На компьютере с сервером включите инструмент YAP Sync.'}</p>
            </div>
            <Button icon='Search' pending={scanning} disabled={busy}
              onClick={() => void findPeers()}>Найти в сети</Button>
          </div>
          {scanError && <p className='kit-error' role='alert'>{scanError}</p>}
          {peers && !peers.length && <p role='status'>Серверы в локальной сети не найдены.</p>}
          {!!peers?.length && <Select label='Найденные серверы' value={peer?.address ?? ''}
            options={peers.map((server) => ({ value: server.address, label: `${server.name} · ${server.address}` }))}
            onChange={(address) => setPeer(peers.find((server) => server.address === address))}
            disabled={busy || scanning} placeholder='Выберите найденный сервер' />}
          {connecting && <p>После разрешения на сервере начнётся обмен треками из выбранной папки.</p>}
        </div>
        <div className='settings-row'>
          <div className='settings-row-info'>
            <h3>Автоматическая синхронизация</h3>
            <p>При запуске YAP и каждые две минуты, пока приложение открыто.</p>
          </div>
          <Switch label='Автоматическая синхронизация' checked={automatic} onChange={setAutomatic} disabled={busy} />
        </div>
      </div>
      <div className='sync-actions'>
        {connecting ? <Button variant='primary' pending={configuring} disabled={busy || !ready}
          onClick={() => void save()}>Подключиться</Button> : <>
          <Button variant='primary' icon='Refresh' pending={running || awaitingApproval}
            disabled={!config || dirty || busy || !ready || awaitingApproval} onClick={runLibrarySync}>Синхронизировать</Button>
          <Button pending={configuring} disabled={busy || !dirty || !ready}
            onClick={() => void save()}>Сохранить настройки</Button>
        </>}
        {canCancel && <Button pending={cancelling} onClick={cancelLibrarySync}>Остановить</Button>}
        {config && <Button variant='ghost' disabled={busy}
          onClick={() => void configureLibrarySync(null)}>Отключить</Button>}
      </div>
    </> : !error && <Progress label='Загрузка настроек синхронизации' />}
    {error && <div className='sync-feedback' role='alert'>
      <p>{error}</p>
      {!status && <Button size='small' onClick={loadLibrarySync}>Повторить</Button>}
    </div>}
    {awaitingApproval && <div className='sync-feedback' role='status'>
      <p>Запрос отправлен. Разрешите доступ на компьютере с сервером.</p>
    </div>}
    {running && <div className='sync-feedback' role='status'>
      <p>{progress ? phaseLabels[progress.phase] : 'Подготовка синхронизации'}</p>
      <Progress label='Ход синхронизации' value={progress?.current} max={progress?.total} counts />
    </div>}
    {cancelled && <p className='sync-report-caption' role='status'>Синхронизация остановлена. Следующий запуск продолжит обмен файлами.</p>}
    {report && <div className='sync-report'>
      <header>
        <div>
          <h3>Последняя синхронизация</h3>
          <time dateTime={new Date(report.finishedAt * 1000).toISOString()}>{date.format(report.finishedAt * 1000)}</time>
        </div>
        <span className='sync-elapsed'>{report.elapsedMs < 1_000
          ? `${number.format(report.elapsedMs)} мс`
          : `${number.format(report.elapsedMs / 1_000)} с`}</span>
      </header>
      <dl className='sync-metrics'>
        <div><dt>Найдено других устройств</dt><dd>{number.format(report.peers)}</dd></div>
        <div><dt>Новые треки</dt><dd>{number.format(report.imported)}</dd></div>
        <div><dt>Совпадения файлов</dt><dd>{number.format(report.duplicates)}</dd></div>
        <div><dt>Обновлено плейлистов</dt><dd>{number.format(report.playlists)}</dd></div>
        <div><dt>Отправлено / получено</dt><dd>{formatBytes(report.bytesSent + report.manifestBytes)} / {formatBytes(report.bytesReceived)}</dd></div>
        <div><dt>Средняя скорость копирования</dt><dd>{report.transferMs > 0
          ? `${formatBytes((report.bytesSent + report.bytesReceived) * 1_000 / report.transferMs)}/с`
          : '—'}</dd></div>
      </dl>
      <p className='sync-report-caption'>Сверено локальных треков: {number.format(report.tracks)} · Прочитано для вычисления хешей: {formatBytes(report.bytesHashed)}</p>
      {Object.keys(report.warnings).length > 0 && <ul className='sync-warnings' aria-label='Проблемы последней синхронизации'>
        {Object.entries(report.warnings).map(([key, count]) => <li key={key}>{warningLabels[key] ?? 'Ошибка обмена'} · {number.format(count)}</li>)}
      </ul>}
    </div>}
  </section>;
}
