import { create } from 'zustand';
import { invalidateCatalog } from '@/hooks/useLibrary';
import { cancelYtDlpDownload, runYtDlpDownload, type MusicDownloadProgress, YtDlpError } from '@/utils/ytDlp';

type ActiveDownload = {
  id: string;
  progress: MusicDownloadProgress | null;
  cancelling: boolean;
};

export const useMusicDownload = create<{ active: ActiveDownload | null }>(() => ({ active: null }));

let nextDownload = 0;

const updateDownload = (id: string, update: Partial<ActiveDownload>) => {
  useMusicDownload.setState(({ active }) => active?.id === id ? { active: { ...active, ...update } } : {});
};

export const downloadMusic = async (videoId: string, folderIndex: number) => {
  if (useMusicDownload.getState().active) throw new YtDlpError('downloadBusy');
  const id = `${Date.now()}-${++nextDownload}`;
  useMusicDownload.setState({ active: { id, progress: null, cancelling: false } });
  try {
    const result = await runYtDlpDownload(id, videoId, folderIndex, (progress) => {
      if (useMusicDownload.getState().active?.progress?.phase === 'processing' && progress.phase === 'downloading') return;
      updateDownload(id, { progress });
    });
    invalidateCatalog();
    return result;
  } finally {
    useMusicDownload.setState({ active: null });
  }
};

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
