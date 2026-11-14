import { create } from 'zustand';
import { Channel, convertFileSrc, invoke } from '@tauri-apps/api/core';
import { invalidateCatalog, tracksByPaths } from '@/hooks/useLibrary';
import { playDownloadedTrack } from '@/stores/playerStore';
import { metadataResolverConfig, useSettingsStore } from '@/stores/settingsStore';
import type { Track } from '@/types/Track';
import { getLocalLyrics, isSingleTimedLine, lookupLyricaLyrics, readProviderLyrics, saveProviderLyrics } from '@/utils/lyrica';
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

export const downloadAndPlayMusic = (videoId: string, folderIndex: number, withVideo = false) =>
  download(videoId, async (id, report) => {
    const result = await runYtDlpDownload(id, videoId, folderIndex, report, withVideo);
    invalidateCatalog();
    const { track } = result;
    if (await playDownloadedTrack(track)) {
      void enrichDownloadedTrack(track).catch((error) => console.warn('Could not find track lyrics', error));
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

async function enrichDownloadedTrack(track: Track) {
  const resolver = metadataResolverConfig();
  const metadata = resolver.mode === 'off' ? Promise.resolve() : invoke('library_enrich_download', { trackId: track.id, resolver })
    .catch((error) => console.warn('Could not enrich downloaded track metadata', error));
  if (!useSettingsStore.getState().lyricaEnabled) return;
  const local = await getLocalLyrics(track.path);
  if (!local || local.sidecar || [local.fetched, track.lyrics.join('\n')].some((text) => text?.trim() && !isSingleTimedLine(text))) return;

  const search = (item: Track) => {
    const artist = item.artists.join(', ').trim();
    return artist && useSettingsStore.getState().lyricaEnabled ? lookupLyricaLyrics(item.name, artist, 'auto') : Promise.resolve(null);
  };
  let result = await search(track);
  if (!result && resolver.mode !== 'off') {
    await metadata;
    const [updated] = await tracksByPaths([track.path]);
    if (updated && (updated.name !== track.name || updated.artists.join(', ') !== track.artists.join(', '))) {
      result = await search(updated);
    }
  }
  if (result && useSettingsStore.getState().lyricaEnabled) await saveProviderLyrics(track.path, readProviderLyrics(result), local.fetched);
}

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
