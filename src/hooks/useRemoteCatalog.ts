import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';
import { useMetadataLookup } from '@/hooks/useLibrary';
import { metadataResolverConfig, useSettingsStore } from '@/stores/settingsStore';
import type { Track } from '@/types/Track';

export type ReleaseKind = 'album' | 'single' | 'ep' | 'other';
export type ReleaseReference = { provider: 'itunes' | 'musicBrainz' | 'deezer' | 'jioSaavn'; id: string };

export type ReleaseSummary = {
  reference: ReleaseReference;
  alternates?: ReleaseReference[];
  title: string;
  kind: ReleaseKind;
  date: string | null;
  artwork: string | null;
};

export type AlbumRelease = ReleaseSummary & { artist: string; artistId?: string };

export type ArtistProfile = {
  avatar: string | null;
  biography: string | null;
  location: string | null;
  activeSince: string | null;
  genres: string[];
  releases: ReleaseSummary[];
};

export type ReleaseTrack = {
  title: string;
  artist: string;
  disc: number | null;
  position: number | null;
  durationMs: number | null;
};

export type Release = {
  title: string;
  artist: string;
  date: string | null;
  artwork: string | null;
  genres?: string[];
  tracks: ReleaseTrack[];
  complete: boolean;
};

function useRemoteCatalog<T>(command: 'library_artist_profile' | 'library_artist_avatar' | 'library_release_details' | 'library_find_release' | 'library_find_album_artwork',
  params: Record<string, unknown> | undefined, useSavedWhenOffline = false) {
  const configKey = useSettingsStore((state) => JSON.stringify([
    state.metadataSource, state.resolverMode, state.resolverUrl, state.resolverSources, state.disabledTools, state.geniusConfigured,
  ]));
  const enabled = useSettingsStore((state) => useSavedWhenOffline || state.metadataSource !== 'off') && params !== undefined;
  const revision = useMetadataLookup((state) => state.releaseRevision);
  const key = JSON.stringify([command, params, configKey]);
  const [attempt, retry] = useState(0);
  const [result, setResult] = useState<{ key: string; attempt: number; payload?: T | null; error?: string }>();

  useEffect(() => {
    if (!enabled) return;
    let active = true;
    const args = { ...params, config: metadataResolverConfig() };
    const refreshProfile = command === 'library_artist_profile';
    const load = async () => {
      if (refreshProfile) {
        const saved = await invoke<T | null>(command, { ...args, savedOnly: true }).catch(() => null);
        if (!active) return;
        if (saved) setResult({ key, attempt: -1, payload: saved });
      }
      const payload = await invoke<T | null>(command, args);
      if (active) setResult((previous) => ({ key, attempt,
        payload: refreshProfile && payload === null && previous?.key === key ? previous.payload ?? null : payload }));
    };
    void load().catch((error) => {
      if (active) setResult((previous) => ({ key, attempt, error: String(error),
        payload: refreshProfile && previous?.key === key ? previous.payload : undefined }));
    });
    return () => { active = false; };
  }, [key, enabled, attempt, revision]);

  return {
    data: enabled && result?.key === key ? result.payload : undefined,
    error: enabled && result?.key === key && result.attempt === attempt ? result.error : undefined,
    loading: enabled && (result?.key !== key || result.attempt !== attempt),
    retry: () => retry((attempt) => attempt + 1),
  };
}

export const useArtistProfile = (name: string, sampleTitles: string[]) =>
  useRemoteCatalog<ArtistProfile>('library_artist_profile', { name, sampleTitles });

export const useArtistAvatar = (name?: string) =>
  useRemoteCatalog<string>('library_artist_avatar', name ? { name } : undefined, true);

export const useReleaseDetails = (item?: ReleaseSummary) =>
  useRemoteCatalog<Release>('library_release_details', item && { reference: item.reference, alternates: item.alternates });

export const useFindAlbumArtwork = (params?: {
  artist: string; title: string; year?: string; kind?: ReleaseKind; references?: ReleaseReference[];
  sample?: ReturnType<typeof releaseSample>; excluded?: string[];
}) =>
  useRemoteCatalog<string>('library_find_album_artwork', params);

export function releaseSample(track: Track | undefined) {
  const artist = track?.artists.join(', ').trim() || track?.albumArtist?.trim() || '';
  const title = track?.name?.trim() ?? '';
  return artist && title ? { artist, title, album: track?.album?.trim() || null } : undefined;
}

export const useTrackRelease = (track: Track | undefined) =>
  useRemoteCatalog<Release>('library_find_release', releaseSample(track));
