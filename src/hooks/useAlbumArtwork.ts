import { useState } from 'preact/hooks';
import { releaseSample, useFindAlbumArtwork, type ReleaseKind, type ReleaseSummary } from '@/hooks/useRemoteCatalog';
import { useCatalogCover, useTrackCover } from '@/hooks/useTrackCover';
import { useSettingsStore } from '@/stores/settingsStore';
import type { Track } from '@/types/Track';

const mbid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu;

export function useAlbumArtwork({ track, albumId, release, artwork, artist, title, year, kind, waitingForDetails, visible = true }: {
  track?: Track;
  albumId?: string;
  release?: ReleaseSummary;
  artwork?: string | null;
  artist?: string;
  title?: string;
  year?: string;
  kind?: ReleaseKind;
  waitingForDetails?: boolean;
  visible?: boolean;
}) {
  const remoteEnabled = useSettingsStore((state) => state.metadataSource !== 'off');
  const local = useTrackCover(visible ? track?.path : undefined);
  const [failed, setFailed] = useState<Set<string>>(() => new Set());
  const references = release ? [release.reference, ...(release.alternates ?? [])] : [];
  const archive = references.find(({ provider, id }) => provider === 'musicBrainz' && mbid.test(id));
  const canLoadRemote = visible && remoteEnabled && !local.loading && !local.coverSrc;
  const direct = canLoadRemote ? [artwork, release?.artwork,
    archive && `https://coverartarchive.org/release-group/${archive.id}/front-500`,
  ].find((url) => url && !failed.has(url)) : undefined;
  const lookup = useFindAlbumArtwork(canLoadRemote && !direct && !waitingForDetails && artist?.trim() && title?.trim()
    ? { artist, title, year: release?.date?.slice(0, 4) ?? year, kind: release?.kind ?? kind,
      references: release ? references : undefined, sample: releaseSample(track), excluded: [...failed] }
    : undefined);
  const found = lookup.data && !failed.has(lookup.data) ? lookup.data : undefined;
  const candidate = direct ?? found;
  const remote = useCatalogCover(candidate, albumId);
  const src = local.coverSrc ?? remote.coverSrc ?? (remote.loading ? undefined : candidate);

  return {
    src,
    loading: local.loading || remote.loading || lookup.loading,
    onError: () => {
      if (local.coverSrc) local.clearCover();
      else if (candidate) setFailed((previous) => new Set([...previous, candidate]));
    },
  };
}
