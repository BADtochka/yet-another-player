import { h } from 'preact';
import { Progress } from '@/components/Progress';
import { create } from 'zustand';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater';
import { logWarn } from '@/utils/appLogs';
import { toast } from '@/utils/toast';

const UPDATE_TOAST_ID = 'app-update';
type AvailableUpdate =
  | { kind: 'standard'; version: string; installer: Update }
  | { kind: 'arch'; version: string };

export const useAvailableUpdate = create<{
  update?: AvailableUpdate;
  installing: boolean;
  progress?: { downloaded: number; total: number };
  pacmanCommand?: string;
}>(() => ({ installing: false }));

const handleDownloadProgress = (
  event: DownloadEvent,
  state: { downloaded: number; total?: number },
) => {
  if (event.event === 'Finished') {
    useAvailableUpdate.setState({ progress: undefined });
    toast.loading('Установка обновления…', { id: UPDATE_TOAST_ID, description: null });
    return;
  }

  if (event.event === 'Started') {
    state.downloaded = 0;
    state.total = event.data.contentLength;
  } else {
    state.downloaded += event.data.chunkLength;
  }

  const total = state.total;
  if (total && total > 0) {
    useAvailableUpdate.setState({ progress: { downloaded: state.downloaded, total } });
    toast.loading('Загрузка обновления…', {
      id: UPDATE_TOAST_ID,
      description: h(Progress, { label: 'Загрузка обновления', value: state.downloaded, max: total }),
    });
  }
};

export const installUpdate = async (update: AvailableUpdate) => {
  if (useAvailableUpdate.getState().installing) return;
  useAvailableUpdate.setState({ installing: true });
  const progress = { downloaded: 0, total: undefined as number | undefined };

  toast.loading('Загрузка обновления…', { id: UPDATE_TOAST_ID });

  try {
    if (update.kind === 'arch') {
      const path = await invoke<string>('download_arch_update', { version: update.version });
      const pacmanCommand = `sudo pacman -U -- '${path.replace(/'/g, "'\\''")}'`;
      useAvailableUpdate.setState({ pacmanCommand });
      toast.message('Пакет скачан и проверен', {
        id: UPDATE_TOAST_ID,
        description: pacmanCommand,
        duration: Number.POSITIVE_INFINITY,
        action: {
          label: 'Показать пакет',
          onClick: () => void revealItemInDir(path).catch((error) => toast.error('Не удалось открыть папку', error)),
        },
      });
      return;
    }

    await update.installer.downloadAndInstall((event) => handleDownloadProgress(event, progress));
    await relaunch();
  } catch (error) {
    if (update.kind === 'arch') useAvailableUpdate.setState({ update: undefined, pacmanCommand: undefined });
    toast.error('Не удалось установить обновление', error, { id: UPDATE_TOAST_ID });
  } finally {
    useAvailableUpdate.setState({ installing: false, progress: undefined });
  }
};

const showUpdateAvailable = (update: AvailableUpdate) => {
  useAvailableUpdate.setState({ update, pacmanCommand: undefined });
  toast.message(`Доступна версия ${update.version}`, {
    id: UPDATE_TOAST_ID,
    duration: Number.POSITIVE_INFINITY,
    description: update.kind === 'arch' ? 'Скачайте пакет и установите его через pacman.' : 'Нажмите «Установить», чтобы обновить приложение.',
    action: {
      label: update.kind === 'arch' ? 'Скачать' : 'Установить',
      onClick: () => {
        void installUpdate(update);
      },
    },
  });
};

const checkForUpdate = async () => {
  if (!import.meta.env.PROD || await getVersion() === '0.0.0') {
    return { status: 'unavailable' as const };
  }

  if (await invoke<boolean>('uses_arch_package')) {
    const version = await invoke<string | null>('check_arch_update');
    if (version) return { status: 'available' as const, update: { kind: 'arch' as const, version } };
  } else {
    const installer = await check();
    if (installer) return { status: 'available' as const, update: { kind: 'standard' as const, version: installer.version, installer } };
  }

  return { status: 'up-to-date' as const };
};

export const checkForAppUpdates = async () => {
  try {
    const result = await checkForUpdate();

    if (result.status === 'available') {
      showUpdateAvailable(result.update);
    }
  } catch (error) {
    logWarn('Automatic update check failed', error);
  }
};

export const checkForAppUpdatesManually = async () => {
  try {
    const result = await checkForUpdate();

    if (result.status === 'unavailable') {
      toast.info('Проверка обновлений доступна в релизной сборке');
      return;
    }

    if (result.status === 'up-to-date') {
      toast.info('Установлена последняя версия');
      return;
    }

    showUpdateAvailable(result.update);
  } catch (error) {
    logWarn('Manual update check failed', error);
    toast.error('Не удалось проверить обновления');
  }
};
