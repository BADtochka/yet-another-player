import { invoke } from '@tauri-apps/api/core';
import { appConfigDir } from '@tauri-apps/api/path';
import { openPath } from '@tauri-apps/plugin-opener';
import { toast } from '@/utils/toast';

type LogLevel = 'error' | 'info' | 'warn';

let globalHandlersInstalled = false;

const serializeDetails = (details: unknown) => {
  if (details instanceof Error) {
    return `${details.name}: ${details.message}\n${details.stack ?? ''}`.trim();
  }

  if (typeof details === 'string') {
    return details;
  }

  try {
    return JSON.stringify(details);
  } catch {
    return String(details);
  }
};

export const getConfigFolderPath = () => appConfigDir();

export const writeAppLog = async (level: LogLevel, message: string, details?: unknown) => {
  const timestamp = new Date().toISOString();
  const detailsText = details === undefined ? '' : ` ${serializeDetails(details)}`;
  const line = `[${timestamp}] [${level.toUpperCase()}] ${message}${detailsText}`;
  await invoke('append_app_log', { line: `${line.slice(0, 32_000)}\n` });
};

export const logInfo = (message: string, details?: unknown) => {
  void writeAppLog('info', message, details).catch((error) => console.warn('Failed to write app log', error));
};

export const logWarn = (message: string, details?: unknown) => {
  void writeAppLog('warn', message, details).catch((error) => console.warn('Failed to write app log', error));
};

export const logError = (message: string, details?: unknown) => {
  void writeAppLog('error', message, details).catch((error) => console.warn('Failed to write app log', error));
};

export const installGlobalLogHandlers = () => {
  if (globalHandlersInstalled) {
    return;
  }

  globalHandlersInstalled = true;
  logInfo('Application started');

  window.addEventListener('error', (event) => {
    logError('Unhandled window error', event.error ?? event.message);
    toast.error('Произошла ошибка', event.error ?? event.message);
  });

  window.addEventListener('unhandledrejection', (event) => {
    logError('Unhandled promise rejection', event.reason);
    toast.error('Произошла ошибка', event.reason);
  });
};

export const openConfigFolder = async () => {
  const configDir = await getConfigFolderPath();

  await openPath(configDir);
};
