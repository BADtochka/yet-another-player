import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';
import { create } from 'zustand';
import { playDownloadedTrack, startTrackClip, stopTrackClip, usePlayerStore } from '@/stores/playerStore';
import { downloadVideo, useMusicDownload } from '@/utils/musicDownload';
import type { Track } from '@/types/Track';
import { toast } from '@/utils/toast';

type VideoRequest = { key: number; path: string; id?: string; src?: string };

export const usePlayerVideo = create<{ request: VideoRequest | null }>(() => ({ request: null }));

let lastRequest = Date.now();

export function closePlayerVideo() {
  const { request } = usePlayerVideo.getState();
  if (!request) return;
  usePlayerVideo.setState({ request: null });
  if (request.src) stopTrackClip(request.key);
}

async function activateClipAudio(request: VideoRequest, cachedId: string | undefined) {
  if (usePlayerVideo.getState().request !== request) return;
  try {
    const started = await startTrackClip(request.key, request.path, cachedId);
    if (usePlayerVideo.getState().request === request && !started) {
      closePlayerVideo();
    }
  } catch (error) {
    if (usePlayerVideo.getState().request === request) {
      closePlayerVideo();
      toast.error('Не удалось включить звук клипа', error);
    }
  }
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
  if (usePlayerStore.getState().currentTrackPath !== track.path && !await playDownloadedTrack(track)) return;
  closePlayerVideo();
  const request = { key: ++lastRequest, path: track.path, id };
  usePlayerVideo.setState({ request });
  try {
    const local = track.hasLocalVideo ? await invoke<string | null>('local_track_video', { path: track.path }) : null;
    const src = local ? convertFileSrc(local) : id ? await downloadVideo(id) : undefined;
    if (!src) throw new Error('Файл клипа недоступен');
    if (usePlayerVideo.getState().request === request) {
      const prepared = { ...request, src };
      usePlayerVideo.setState({ request: prepared });
      await activateClipAudio(prepared, local ? undefined : id);
    }
  } catch (error) {
    if (usePlayerVideo.getState().request === request) {
      closePlayerVideo();
      if (error !== 'downloadCancelled') toast.error('Не удалось загрузить видео', error);
    }
  }
}
