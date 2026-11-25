import { create } from 'zustand';
import { Channel, convertFileSrc, invoke } from '@tauri-apps/api/core';
import { invalidateCatalog } from '@/hooks/useLibrary';
import { metadataResolverConfig } from '@/stores/settingsStore';
import { cancelYtDlpDownload, runYtDlpDownload, type MusicDownloadProgress, YtDlpError } from '@/utils/ytDlp';

type ActiveDownload = {
  id: string;
  videoId: string;
  progress: MusicDownloadProgress | null;
  cancelling: boolean;
};

export const useMusicDownload = create<{ active: ActiveDownload | null }>(() => ({ active: null }));

let nextDownload = 0;

const updateDownload = (id: string, update: Partial<ActiveDownload>) => {
  useMusicDownload.setState(({ active }) => active?.id === id ? { active: { ...active, ...update } } : {});
};

async function download<T>(videoId: string, run: (id: string, report: (progress: MusicDownloadProgress) => void) => Promise<T>) {
  if (useMusicDownload.getState().active) throw new YtDlpError('downloadBusy');
  const id = `${Date.now()}-${++nextDownload}`;
  useMusicDownload.setState({ active: { id, videoId, progress: null, cancelling: false } });
  try {
    return await run(id, (progress) => {
      if (useMusicDownload.getState().active?.progress?.phase === 'processing' && progress.phase === 'downloading') return;
      updateDownload(id, { progress });
    });
  } finally {
    useMusicDownload.setState({ active: null });
  }
}

export const downloadMusic = (videoId: string, folderIndex: number, withVideo = false) =>
  download(videoId, async (id, report) => {
    const result = await runYtDlpDownload(id, videoId, folderIndex, report, withVideo);
    invalidateCatalog();
    const { track, alreadyPresent } = result;
    if (!alreadyPresent) {
      const resolver = metadataResolverConfig();
      if (resolver.primary !== 'off') {
        void invoke('library_enrich_download', { trackId: track.id, resolver })
          .catch((error) => console.warn('Could not enrich downloaded track metadata', error));
      }
    }
    return track;
  });

export const downloadVideo = (videoId: string) =>
  download(videoId, async (id, report) => {
    const progress = new Channel<MusicDownloadProgress>();
    progress.onmessage = report;
    const path = await invoke<string>('download_yt_dlp_video', { id, videoId, progress });
    return convertFileSrc(path);
  });

export const cancelMusicDownload = async () => {
  const { active } = useMusicDownload.getState();
  if (!active?.progress || active.cancelling || active.progress.phase === 'importing') return;
  updateDownload(active.id, { cancelling: true });
  try {
    if (await cancelYtDlpDownload(active.id)) return;
  } catch (error) {
    updateDownload(active.id, { cancelling: false });
    throw error;
  }
  updateDownload(active.id, { cancelling: false });
};
