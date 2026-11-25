import { Channel, invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';

import { invalidateCatalog } from '@/hooks/useLibrary';
import { changePlaylists } from '@/stores/playlistStore';

type SyncConfig = { destination: string; automatic: boolean; invite?: string | null };
type SyncSelection = { folder: string; automatic: boolean; invite: string | null };
type SyncReport = {
  finishedAt: number;
  elapsedMs: number;
  peers: number;
  tracks: number;
  duplicates: number;
  imported: number;
  playlists: number;
  bytesHashed: number;
  bytesSent: number;
  bytesReceived: number;
  transferMs: number;
  manifestBytes: number;
  catalogChanged: boolean;
  warnings: Record<string, number>;
};
type SyncStatus = { config: SyncConfig | null; report: SyncReport | null; address: string | null };
type SyncProgress = {
  phase: 'preparing' | 'indexing' | 'sending' | 'receiving' | 'applying' | 'publishing';
  current: number;
  total: number | null;
};

export const useLibrarySync = create<{
  status: SyncStatus | null;
  running: boolean;
  configuring: boolean;
  cancelling: boolean;
  cancelled: boolean;
  approvalDeadline: number | null;
  progress: SyncProgress | null;
  error: string | null;
}>(() => ({ status: null, running: false, configuring: false, cancelling: false, cancelled: false, approvalDeadline: null, progress: null, error: null }));

const failureCode = (error: unknown) =>
  error && typeof error === 'object' && 'code' in error ? error.code : undefined;

const failureMessage = (error: unknown) =>
  error && typeof error === 'object' && 'message' in error && typeof error.message === 'string'
    ? error.message
    : 'Не удалось выполнить синхронизацию.';

export async function loadLibrarySync() {
  if (useLibrarySync.getState().status) return;
  try {
    useLibrarySync.setState({ status: await invoke<SyncStatus>('library_sync_status'), error: null });
  } catch (error) {
    useLibrarySync.setState({ error: failureMessage(error) });
  }
}

export async function configureLibrarySync(config: SyncSelection | null) {
  const { running, configuring, status } = useLibrarySync.getState();
  if (running || configuring) return false;
  useLibrarySync.setState({ configuring: true, error: null, cancelled: false });
  try {
    const next = await invoke<SyncStatus>('library_sync_configure', { config });
    useLibrarySync.setState({
      status: next,
      approvalDeadline: next.config?.invite === status?.config?.invite ? useLibrarySync.getState().approvalDeadline : null,
    });
    return true;
  } catch (error) {
    useLibrarySync.setState({ error: failureMessage(error) });
    return false;
  } finally {
    useLibrarySync.setState({ configuring: false });
  }
}

export async function runLibrarySync() {
  const { status, running, configuring, approvalDeadline } = useLibrarySync.getState();
  if (!status?.config || running || configuring || approvalDeadline) return;
  useLibrarySync.setState({ running: true, progress: null, error: null, cancelled: false });
  const progress = new Channel<SyncProgress>((progress) => useLibrarySync.setState({ progress }));
  try {
    const report = await invoke<SyncReport | null>('library_sync_run', { progress });
    if (report) useLibrarySync.setState({ status: { ...status, report } });
    if (report?.catalogChanged) {
      invalidateCatalog();
      void changePlaylists({ type: 'list' }).catch((error) => console.warn('Could not reload synchronized playlists', error));
    }
  } catch (error) {
    if (failureCode(error) === 'cancelled') {
      useLibrarySync.setState({ cancelled: true });
    } else if (failureCode(error) === 'pairingPending') {
      useLibrarySync.setState({ approvalDeadline: Date.now() + 14 * 60_000 });
    } else {
      useLibrarySync.setState({ error: failureMessage(error) });
    }
  } finally {
    useLibrarySync.setState({ running: false, cancelling: false, progress: null });
  }
}

export async function cancelLibrarySync() {
  const { running, cancelling } = useLibrarySync.getState();
  if (!running || cancelling) return;
  useLibrarySync.setState({ cancelling: true });
  try {
    await invoke('library_sync_cancel');
  } catch (error) {
    useLibrarySync.setState({ cancelling: false, error: failureMessage(error) });
  }
}

export function startLibrarySync() {
  let active = true;
  let checking = false;
  const tick = () => {
    if (active && useLibrarySync.getState().status?.config?.automatic) void runLibrarySync();
  };
  void loadLibrarySync().then(tick);
  const interval = window.setInterval(tick, 120_000);
  const approvalPoll = window.setInterval(async () => {
    const { status, approvalDeadline, configuring } = useLibrarySync.getState();
    const config = status?.config;
    if (!config?.invite || !approvalDeadline || configuring || checking) return;
    if (Date.now() >= approvalDeadline) {
      useLibrarySync.setState({ approvalDeadline: null, error: 'Время ожидания истекло. Запустите синхронизацию, чтобы проверить доступ.' });
      return;
    }
    const current = () => {
      const state = useLibrarySync.getState();
      return active && state.status?.config === config && state.approvalDeadline === approvalDeadline && !state.configuring;
    };
    checking = true;
    try {
      await invoke('library_sync_check_access', { expectedInvite: config.invite });
      if (current()) {
        useLibrarySync.setState({ approvalDeadline: null });
        void runLibrarySync();
      }
    } catch (error) {
      const code = failureCode(error);
      if (current() && code !== 'pairingPending' && code !== 'server') {
        useLibrarySync.setState({ approvalDeadline: null, error: failureMessage(error) });
      }
    } finally {
      checking = false;
    }
  }, 3_000);
  return () => {
    active = false;
    window.clearInterval(interval);
    window.clearInterval(approvalPoll);
  };
}
