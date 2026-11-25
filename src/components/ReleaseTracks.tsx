import { Link } from '@tanstack/react-router';
import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { MediaCard } from '@/components/MediaCard';
import { TrackArtistLinks } from '@/components/TrackLinks';
import type { Release, ReleaseTrack } from '@/hooks/useRemoteCatalog';
import { useSettingsStore } from '@/stores';
import type { Track } from '@/types/Track';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';

export const titleKey = (value: string) => value.toLowerCase().replace(/ - (?:single|ep)$/iu, '')
  .replace(/ё/gu, 'е').replace(/[^\p{L}\p{N}]/gu, '');

export function releaseTitleKey(value: string) {
  const alternate = value.trim().replace(/ - (?:single|ep)$/iu, '').match(/^(.*)\(([\x20-\x7e]+)\)$/u);
  if (alternate && /[\u0400-\u052f]/u.test(alternate[1]) && /[a-z]/iu.test(alternate[2])
    && !/live|remix|edit|deluxe|remaster|acoustic|version|instrumental|feat|slowed|sped up|demo/iu.test(alternate[2])) {
    return titleKey(alternate[1]);
  }
  return titleKey(value.trim());
}

export function releaseArtistNames(credit: string, knownArtists: ReadonlyMap<string, string>) {
  const exact = knownArtists.get(titleKey(credit));
  if (exact) return [exact];
  const names = credit.split(/,\s*|\s+&\s+/u).map((name) => knownArtists.get(titleKey(name)));
  return names.every((name) => name !== undefined) ? names : [credit];
}

export type AlbumTrack = { key: string; local: Track } | { key: string; remote: ReleaseTrack };

export function mergeReleaseTracks(local: Track[], release?: Release | null): AlbumTrack[] {
  const localRows = local.map((track) => ({ key: `local:${track.id}`, local: track }));
  if (!release?.tracks.length) return localRows;

  const byTitle = new Map<string, typeof localRows>();
  for (const row of localRows) {
    const title = releaseTitleKey(row.local.name);
    const candidates = byTitle.get(title);
    if (candidates) candidates.push(row);
    else byTitle.set(title, [row]);
  }

  const used = new Set<string>();
  const remoteTitles = new Map<string, number>();
  for (const track of release.tracks) {
    const title = releaseTitleKey(track.title);
    remoteTitles.set(title, (remoteTitles.get(title) ?? 0) + 1);
  }
  const ordered = release.tracks.map((remote, index) => {
    const title = releaseTitleKey(remote.title);
    const candidates = (byTitle.get(title) ?? []).filter(({ key, local: track }) =>
      !used.has(key) && (!remote.disc || !track.discNumber || remote.disc === track.discNumber));
    const matching = candidates.find(({ local: track }) => remote.position != null && track.trackNumber === remote.position)
      ?? candidates.find(({ local: track }) => !remote.durationMs || !track.duration || Math.abs(track.duration - remote.durationMs / 1000) <= 4)
      ?? (candidates.length === 1 && remoteTitles.get(title) === 1 ? candidates[0] : undefined);
    if (matching) used.add(matching.key);
    return {
      row: matching ?? { key: `external:${index}`, remote },
      disc: remote.disc ?? 1,
      position: remote.position ?? index + 1,
    };
  });

  for (const row of localRows) {
    if (!used.has(row.key)) {
      ordered.push({ row, disc: row.local.discNumber ?? 1, position: row.local.trackNumber ?? Infinity });
    }
  }
  return ordered.sort((a, b) => a.disc - b.disc || a.position - b.position).map(({ row }) => row);
}

export function ReleaseTrackCard({ track, album, artist, artwork, onArtworkError, linkedArtists }: {
  track: ReleaseTrack;
  album: string;
  artist: string;
  artwork?: string | null;
  onArtworkError: () => void;
  linkedArtists: ReadonlyMap<string, string>;
}) {
  const downloadEnabled = useSettingsStore((state) => !state.disabledTools.includes('yt-dlp'));
  const credit = track.artist || artist;
  const query = [credit, track.title, releaseTitleKey(track.title) === releaseTitleKey(album) ? '' : album]
    .filter(Boolean).join(' ');
  const label = `Найти «${track.title}» для скачивания`;
  return <MediaCard title={track.title} subtitle={credit} icon='Track'
    subtitleLinks={credit ? <TrackArtistLinks artists={releaseArtistNames(credit, linkedArtists)} allowExternal /> : undefined}
    coverSrc={artwork} onCoverError={onArtworkError}
    metadata={track.durationMs ? formatPlaybackTime(track.durationMs / 1000) : undefined}
    action={downloadEnabled
      ? <Link className='kit-button track-row-action' data-size='icon' data-variant='secondary'
        aria-label={label} title={label} to='/search' search={{ q: query, tab: 'tracks', request: undefined }}>
        <><Icon name='Search' /></></Link>
      : <Button className='track-row-action' size='icon' icon='Search' aria-label={label}
        title='Подключите yt-dlp для скачивания' disabled />} />;
}
