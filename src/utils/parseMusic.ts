import { Channel, invoke } from '@tauri-apps/api/core';
import { invalidateCatalog } from '@/hooks/useLibrary';
import { useSettingsStore } from '@/stores/settingsStore';
import { registerMusicRoots } from '@/utils/musicFolders';

export type TrackScanProgress = {
  current: number;
  total: number | null;
};

type Catalog = {
  initialized: boolean;
  trackCount: number;
};

type ScanResult = {
  cancelled: boolean;
  unavailableFolders: number;
  catalog: Catalog;
};

type ScanOptions = {
  id?: string;
  folders?: string[];
  onProgress?: (progress: TrackScanProgress) => void;
};

let scanCounter = 0;
let scanActive = false;
export const createScanId = () => `${Date.now()}-${++scanCounter}`;

export const cancelMusicScan = (id: string) => invoke<void>('library_cancel', { id });

export const parseMusic = async ({ id = createScanId(), folders, onProgress }: ScanOptions = {}) => {
  if (scanActive) throw new Error('Сканирование уже запущено.');
  scanActive = true;

  const registeredFolders = useSettingsStore.getState().musicFolders;
  try {
    await registerMusicRoots(folders ?? registeredFolders);
    const progress = new Channel<TrackScanProgress>();
    progress.onmessage = (message) => onProgress?.(message);
    const result = await invoke<ScanResult>('library_scan', { id, progress });

    if (result.cancelled) {
      await registerMusicRoots(registeredFolders);
    } else {
      invalidateCatalog();
    }
    return result;
  } catch (error) {
    await registerMusicRoots(registeredFolders).catch(() => undefined);
    throw error;
  } finally {
    scanActive = false;
  }
};

export const loadCatalog = async () => {
  const catalog = await invoke<Catalog>('library_catalog_status');

  if (!catalog.initialized && useSettingsStore.getState().musicFolders.length > 0) {
    await parseMusic();
  }
};
