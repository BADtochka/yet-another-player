import { Channel, invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import { type Track } from '@/types/Track';
import { logError } from '@/utils/appLogs';

export type MusicSearchResult = {
  id: string;
  title: string;
  artist: string | null;
  duration: number | null;
  thumbnail: string | null;
};

export const ytDlpFailureMessage = {
  missing: 'Установите yt-dlp в настройках YAP.',
  timedOut: 'yt-dlp не ответил за 45 секунд. Попробуйте ещё раз.',
  gatekeeperBlocked: 'macOS заблокировала запуск yt-dlp.',
  launchFailed: 'Не удалось запустить yt-dlp. Попробуйте ещё раз или переустановите инструмент в настройках YAP.',
  requestFailed: 'Не удалось получить результаты с YouTube. Попробуйте ещё раз.',
  invalidQuery: 'Проверьте длину поискового запроса.',
  invalidVideo: 'У этой карточки нет допустимого идентификатора YouTube.',
  previewFailed: 'Не удалось подготовить аудио для предпросмотра.',
  previewCancelled: 'Предпросмотр отменён.',
  previewTooLarge: 'Аудио превышает лимит предпросмотра в 32 МБ.',
  downloadBusy: 'Другая загрузка уже выполняется.',
  invalidDownload: 'Не удалось запустить загрузку. Попробуйте ещё раз.',
  downloadCancelled: 'Загрузка отменена.',
  downloadTimedOut: 'Загрузка не завершилась за отведённое время.',
  downloadFailed: 'yt-dlp не смог скачать аудио. Попробуйте ещё раз.',
  ageRestricted: 'YouTube просит подтвердить возраст для этого видео.',
  targetUnavailable: 'Выбранная папка медиатеки недоступна для записи.',
  importFailed: 'Не удалось добавить скачанный файл в медиатеку.',
  cleanupFailed: 'Не удалось полностью удалить временные файлы загрузки.',
};

type YtDlpFailure = keyof typeof ytDlpFailureMessage;
type Ready = { state: 'ready'; version: string };
type YtDlpCheck = Ready | { state: YtDlpFailure };

export type MusicDownloadProgress =
  | { phase: 'preparing' | 'processing' | 'waitingForLibrary' | 'importing' }
  | { phase: 'downloading'; percent: number | null };

export class YtDlpError extends Error {
  constructor(readonly state: YtDlpFailure, message = ytDlpFailureMessage[state]) {
    super(message);
  }
}

export const useYtDlpStatus = create<{
  checking: boolean;
  gatekeeperDialogOpen: boolean;
}>(() => ({ checking: false, gatekeeperDialogOpen: false }));

let pendingCheck: Promise<YtDlpCheck> | undefined;
let verifiedVersion: Ready | undefined;
export const clearYtDlpCheck = () => { verifiedVersion = undefined; };

export const checkYtDlp = (fresh = false): Promise<YtDlpCheck> => {
  if (pendingCheck) {
    return fresh ? pendingCheck.then(() => checkYtDlp(true)) : pendingCheck;
  }
  if (fresh) verifiedVersion = undefined;
  if (verifiedVersion) return Promise.resolve(verifiedVersion);

  useYtDlpStatus.setState({ checking: true });
  pendingCheck = invoke<string>('check_yt_dlp')
    .then((version): Ready => {
      verifiedVersion = { state: 'ready', version };
      return verifiedVersion;
    })
    .catch((error): YtDlpCheck => {
      return { state: handleYtDlpFailure(error) };
    })
    .finally(() => {
      pendingCheck = undefined;
      useYtDlpStatus.setState({ checking: false });
    });
  return pendingCheck;
};

const handleYtDlpFailure = (error: unknown): YtDlpFailure => {
  if (typeof error !== 'string' || !Object.prototype.hasOwnProperty.call(ytDlpFailureMessage, error)) {
    logError('yt-dlp IPC failed', error);
    error = 'launchFailed';
  }
  const state = error as YtDlpFailure;
  if (state === 'missing' || state === 'launchFailed' || state === 'gatekeeperBlocked') {
    verifiedVersion = undefined;
  }
  if (state === 'gatekeeperBlocked') {
    useYtDlpStatus.setState({ gatekeeperDialogOpen: true });
  }
  return state;
};

export const getYtDlpLocation = async () => {
  try {
    return await invoke<string>('get_yt_dlp_location');
  } catch (error) {
    throw new YtDlpError(handleYtDlpFailure(error));
  }
};

export const runYtDlpSearch = async (query: string) => {
  try {
    return await invoke<MusicSearchResult[]>('search_yt_dlp', { query });
  } catch (error) {
    const state = handleYtDlpFailure(error);
    throw new YtDlpError(state, state === 'timedOut' ? 'YouTube Music не ответил вовремя. Повторите поиск.' : undefined);
  }
};

export const startMusicPreview = async (id: number, videoId: string) => {
  try {
    await invoke<void>('preview_yt_dlp', { id, videoId });
  } catch (error) {
    const state = handleYtDlpFailure(error);
    throw new YtDlpError(state, state === 'timedOut' ? 'Предпросмотр не загрузился вовремя.' : undefined);
  }
};

export const runYtDlpDownload = async (
  id: string,
  videoId: string,
  folderIndex: number,
  onProgress: (progress: MusicDownloadProgress) => void,
) => {
  const progress = new Channel<MusicDownloadProgress>();
  progress.onmessage = onProgress;
  try {
    return await invoke<{ track: Track; alreadyPresent: boolean }>('download_yt_dlp', {
      id, videoId, folderIndex, progress,
    });
  } catch (error) {
    throw new YtDlpError(handleYtDlpFailure(error));
  }
};

export const cancelYtDlpDownload = (id: string) => invoke<boolean>('cancel_yt_dlp_download', { id });

export const openMacosPrivacySettings = () => invoke<void>('open_macos_privacy_settings');

export const dismissGatekeeperDialog = () => useYtDlpStatus.setState({ gatekeeperDialogOpen: false });
