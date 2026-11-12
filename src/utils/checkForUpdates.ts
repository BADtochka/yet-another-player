import { create } from 'zustand';
import { getVersion } from '@tauri-apps/api/app';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater';
import { logWarn } from '@/utils/appLogs';
import { toast } from '@/utils/toast';

const UPDATE_TOAST_ID = 'app-update';
export const useAvailableUpdate = create<{ update?: Update; installing: boolean }>(() => ({ installing: false }));

const formatDownloadProgress = (downloaded: number, total?: number) => {
  if (total && total > 0) {
    const percent = Math.min(100, Math.round((downloaded / total) * 100));
    return `Загрузка обновления... ${percent}%`;
  }

  const megabytes = (downloaded / 1024 / 1024).toFixed(1);
  return `Загрузка обновления... ${megabytes} МБ`;
};

const handleDownloadProgress = (
  event: DownloadEvent,
  state: { downloaded: number; total?: number },
) => {
  if (event.event === 'Started') {
    state.downloaded = 0;
    state.total = event.data.contentLength;
    toast.loading(formatDownloadProgress(state.downloaded, state.total), { id: UPDATE_TOAST_ID });
    return;
  }

  if (event.event === 'Progress') {
    state.downloaded += event.data.chunkLength;
    toast.loading(formatDownloadProgress(state.downloaded, state.total), { id: UPDATE_TOAST_ID });
    return;
  }

  if (event.event === 'Finished') {
    toast.loading('Установка обновления...', { id: UPDATE_TOAST_ID });
  }
};

export const installUpdate = async (update: Update) => {
  if (useAvailableUpdate.getState().installing) return;
  useAvailableUpdate.setState({ installing: true });
  const progress = { downloaded: 0, total: undefined as number | undefined };

  toast.loading('Загрузка обновления...', { id: UPDATE_TOAST_ID });

  try {
    await update.downloadAndInstall((event) => handleDownloadProgress(event, progress));
    await relaunch();
  } catch (error) {
    toast.error('Не удалось установить обновление', error, { id: UPDATE_TOAST_ID });
  } finally {
    useAvailableUpdate.setState({ installing: false });
  }
};

const showUpdateAvailable = (update: Update) => {
  useAvailableUpdate.setState({ update });
  toast.message(`Доступна версия ${update.version}`, {
    id: UPDATE_TOAST_ID,
    duration: Number.POSITIVE_INFINITY,
    description: 'Нажмите «Установить», чтобы обновить приложение.',
    action: {
      label: 'Установить',
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

  const update = await check();

  if (!update) {
    return { status: 'up-to-date' as const };
  }

  return { status: 'available' as const, update };
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
