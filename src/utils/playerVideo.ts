import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';
import { create } from 'zustand';
import { playDownloadedTrack, usePlayerStore } from '@/stores/playerStore';
import { downloadVideo, useMusicDownload } from '@/utils/musicDownload';
import type { Track } from '@/types/Track';
import { toast } from '@/utils/toast';

type VideoRequest = { path: string; id?: string; src?: string };

export const usePlayerVideo = create<{ request: VideoRequest | null }>(() => ({ request: null }));

export function closePlayerVideo() {
  usePlayerVideo.setState({ request: null });
}

const availability = new Map<string, { expires: number; result: Promise<boolean> }>();

export function checkPlayerVideo(id: string) {
  const cached = availability.get(id);
  if (cached && cached.expires > Date.now()) return cached.result;

  const result = invoke<boolean>('external_video_available', { videoId: id }).catch(() => false);
  availability.set(id, { expires: Date.now() + 60_000, result });
  return result;
}

export function useVideoAvailable(id?: string) {
  const [confirmed, setConfirmed] = useState<string>();
  useEffect(() => {
    let active = true;
    if (id) void checkPlayerVideo(id).then((available) => {
      if (active) setConfirmed(available ? id : undefined);
    });
    return () => { active = false; };
  }, [id]);
  return !!id && confirmed === id;
}

export function trackVideoId(track?: Track | null) {
  return track?.clipVideoId;
}

export async function showPlayerVideo(track: Track) {
  const id = trackVideoId(track);
  if ((!id && !track.hasLocalVideo) || useMusicDownload.getState().active) return;
  const player = usePlayerStore.getState();
  if (player.currentTrackPath !== track.path && !await playDownloadedTrack(track)) return;
  const request = { path: track.path, id };
  usePlayerVideo.setState({ request });
  try {
    const local = track.hasLocalVideo ? await invoke<string | null>('local_track_video', { path: track.path }) : null;
    const src = local ? convertFileSrc(local) : id ? await downloadVideo(id) : undefined;
    if (!src) throw new Error('Файл клипа недоступен');
    if (usePlayerVideo.getState().request === request) usePlayerVideo.setState({ request: { ...request, src } });
  } catch (error) {
    if (usePlayerVideo.getState().request === request) {
      closePlayerVideo();
      if (error !== 'downloadCancelled') toast.error('Не удалось загрузить видео', error);
    }
  }
}
