import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';

type DeviceAccess = { id: string; name: string; status: 'pending' | 'allowed' | 'blocked'; local: boolean };

export const useSyncServerAccess = create<{
  devices?: DeviceAccess[];
  action?: { id: string; allowed: boolean };
  loadError?: string;
  accessError?: string;
}>(() => ({}));

let revision = 0;

export function startSyncServerAccess() {
  let active = true;
  let loading = false;
  const refresh = async () => {
    if (loading || useSyncServerAccess.getState().action) return;
    loading = true;
    const requested = revision;
    try {
      const devices = await invoke<DeviceAccess[]>('sync_server_devices');
      if (active && requested === revision) useSyncServerAccess.setState({ devices, loadError: undefined });
    } catch (error) {
      if (active && requested === revision) useSyncServerAccess.setState({ loadError: String(error) });
    } finally {
      loading = false;
    }
  };
  void refresh();
  const poll = window.setInterval(refresh, 2_000);
  return () => {
    active = false;
    window.clearInterval(poll);
  };
}

export async function setSyncServerAccess(id: string, allowed: boolean) {
  if (useSyncServerAccess.getState().action) return false;
  revision += 1;
  useSyncServerAccess.setState({ action: { id, allowed }, accessError: undefined });
  try {
    await invoke('sync_server_set_access', { id, allowed });
    useSyncServerAccess.setState(({ devices }) => ({
      devices: devices?.map((device) => device.id === id
        ? { ...device, status: allowed ? 'allowed' : 'blocked' }
        : device),
    }));
    return true;
  } catch (error) {
    useSyncServerAccess.setState({ accessError: String(error) });
    return false;
  } finally {
    useSyncServerAccess.setState({ action: undefined });
  }
}
