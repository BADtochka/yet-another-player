import { Link } from '@tanstack/react-router';
import { useId, useLayoutEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Artwork } from '@/components/MediaCard';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { releaseTitleKey, titleKey } from '@/components/ReleaseTracks';
import { Track } from '@/components/Track';
import { useArtistProfile, type AlbumRelease, type ReleaseKind } from '@/hooks/useRemoteCatalog';
import { useAlbumArtwork } from '@/hooks/useAlbumArtwork';
import { useIntersection } from '@/hooks/useIntersection';
import { useCatalogCover } from '@/hooks/useTrackCover';
import { usePlayerStore } from '@/stores';
import type { Track as LibraryTrack } from '@/types/Track';
import { catalogNames, type Collection, formatTrackCount } from '@/utils/collections';

const releaseGroups = [
  { id: 'albums', kinds: ['album'], title: 'Альбомы' },
  { id: 'singles', kinds: ['single', 'ep'], title: 'Синглы и EP' },
  { id: 'other', kinds: ['other'], title: 'Другие релизы' },
] as const;

type LocalRelease = { id: string; name: string; kind: ReleaseKind; date?: string; tracks: LibraryTrack[] };
type DiscographyRelease = {
  key: string;
  title: string;
  kind: ReleaseKind;
  date?: string | null;
  collection?: string;
  track?: LibraryTrack;
  release?: AlbumRelease;
};

function sameRelease(name: string, date: string | null | undefined, item: LocalRelease) {
  return releaseTitleKey(name) === releaseTitleKey(item.name)
    && (!date || !item.date || date.slice(0, 4) === item.date.slice(0, 4));
}

function localReleases(tracks: LibraryTrack[]): LocalRelease[] {
  const albums = new Map<string, LibraryTrack[]>();
  for (const track of tracks) {
    const album = albums.get(track.albumId);
    if (album) album.push(track);
    else albums.set(track.albumId, [track]);
  }
  const releases: LocalRelease[] = [];
  for (const [id, album] of albums) {
    const [track] = album;
    const name = track.album?.trim() || track.name;
    const kind = album.some((track) => track.isEp) || / - ep$/iu.test(name) ? 'ep'
      : track.isSingle || id.startsWith('single:') || / - single$/iu.test(name)
        || (album.length === 1 && releaseTitleKey(name) === releaseTitleKey(track.name)) ? 'single' : 'album';
    const date = track.releaseDate ?? (track.year ? String(track.year) : undefined);
    const existing = releases.find((item) => sameRelease(name, date, item)
      && (item.kind === kind || titleKey(name) !== titleKey(item.name)));
    if (!existing) {
      releases.push({ id, name, kind, date, tracks: album });
    } else {
      if (album.length > existing.tracks.length || (album.length === existing.tracks.length && name.length < existing.name.length)) {
        Object.assign(existing, { id, name, date, kind });
      }
      for (const candidate of album) {
        if (!existing.tracks.some((track) => releaseTitleKey(track.name) === releaseTitleKey(candidate.name)
          && (track.discNumber ?? 1) === (candidate.discNumber ?? 1)
          && (!track.trackNumber || !candidate.trackNumber || track.trackNumber === candidate.trackNumber))) {
          existing.tracks.push(candidate);
        }
      }
    }
  }
  return releases;
}

function ReleaseArtwork({ item, artist }: { item: DiscographyRelease; artist: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  const visible = useIntersection(ref);
  const { src, onError, loading } = useAlbumArtwork({
    release: item.release, track: item.track, albumId: item.collection,
    artist: item.track?.albumArtist?.trim() || artist,
    title: item.title, year: item.date?.slice(0, 4), kind: item.kind, visible,
  });
  return <span ref={ref}><Artwork icon='Album' src={src} onError={onError} pending={loading} /></span>;
}

function DiscographySection({ title, artist, releases }: { title: string; artist: string; releases: DiscographyRelease[] }) {
  const gridRef = useRef<HTMLDivElement>(null);
  const gridId = useId();
  const [columns, setColumns] = useState(1);
  const [expanded, setExpanded] = useState(false);

  useLayoutEffect(() => {
    const grid = gridRef.current;
    if (!grid) return;
    const measure = () => {
      // The auto-fill grid includes empty columns in its computed track list.
      setColumns(getComputedStyle(grid).gridTemplateColumns.split(/\s+/u).length);
    };
    measure();
    let width: number | undefined;
    let frame = 0;
    const observer = new ResizeObserver(([entry]) => {
      if (entry.contentRect.width === width) return;
      width = entry.contentRect.width;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    });
    observer.observe(grid);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, []);

  const previewCount = columns * 2;
  const canExpand = releases.length > previewCount + 1;
  const visibleReleases = canExpand && !expanded ? releases.slice(0, previewCount) : releases;

  return <section className='artist-releases'>
    <h2>{title}</h2>
    <div id={gridId} ref={gridRef} className='artist-release-grid'>
      {visibleReleases.map((item) => <Link className='artist-release'
        key={item.key} to='/albums' search={{ collection: item.collection, release: item.release }}>
        <ReleaseArtwork item={item} artist={artist} />
        <strong>{item.title}</strong>
        {item.date && <small>{item.date.slice(0, 4)}</small>}
      </Link>)}
    </div>
    {canExpand && <Button size='small' aria-controls={gridId} aria-expanded={expanded}
      onClick={() => setExpanded(!expanded)}>{expanded ? 'Свернуть' : 'Показать все'}</Button>}
  </section>;
}

export function ArtistProfilePage({ artist }: { artist: Collection }) {
  const samples = [...new Set(artist.tracks.map((track) => track.name))].slice(0, 4);
  const { data: profile, loading, error, retry } = useArtistProfile(artist.name, samples);
  const avatar = useCatalogCover(profile?.avatar ?? undefined, undefined, true);
  const [trackLimit, setTrackLimit] = useState(24);
  const play = usePlayerStore((state) => state.playTrack);
  const remote = profile?.releases ?? [];
  const local = localReleases(artist.tracks);
  const releasesWithLocal = remote.map((release) => {
    const matching = local.filter((item) => sameRelease(release.title, release.date, item));
    const match = matching.find((item) => item.kind === release.kind) ?? (matching.length === 1 ? matching[0] : undefined);
    const kind = release.kind === 'album' && (match?.kind === 'ep' || (match?.kind === 'single' && match.tracks[0]?.isSingle === true))
      ? match.kind : release.kind;
    return { release: { ...release, kind }, local: match };
  });
  const matched = new Set(releasesWithLocal.flatMap(({ local }) => local ? [local.id] : []));
  const discography: DiscographyRelease[] = [
    ...releasesWithLocal.map(({ release, local }) => ({
      key: `${release.reference.provider}:${release.reference.id}`,
      title: release.title,
      date: local?.date && local.date.length > (release.date?.length ?? 0) ? local.date : release.date,
      kind: release.kind,
      collection: local?.id,
      track: local?.tracks[0],
      release: { ...release, artist: artist.name, artistId: artist.id },
    })),
    ...local.filter((item) => !matched.has(item.id)).map((item) => ({
      key: item.id,
      title: item.name,
      date: item.date,
      kind: item.kind,
      collection: item.id,
      track: item.tracks[0],
      release: undefined,
    })),
  ].sort((left, right) => (right.date ?? '').localeCompare(left.date ?? '')
    || catalogNames.compare(left.title, right.title));
  return <article className='artist-page'>
    <header className='artist-profile-header'>
      <Artwork className='artist-avatar' icon='User' src={avatar.coverSrc} onError={avatar.clearCover} />
      <div className='artist-profile-info'>
        <div className='artist-profile-title'>
          <h1>{artist.name}</h1>
          {loading && <LoadingIndicator label='Обновление профиля и дискографии' />}
        </div>
        <p>{formatTrackCount(artist.tracks.length)} в медиатеке</p>
        <div className='artist-facts'>
          {profile?.location && <span>{profile.location}</span>}
          {profile?.activeSince && <span>С {profile.activeSince.slice(0, 4)} года</span>}
          {profile?.genres.map((genre) => <span key={genre}>{genre}</span>)}
        </div>
        {error && <div className='artist-profile-status'>
          <p role='status'>Не удалось обновить дискографию.</p>
          <Button size='small' onClick={retry}>Повторить</Button>
        </div>}
      </div>
    </header>
    {profile?.biography && <section className='artist-bio'><h2>Об исполнителе</h2><p>{profile.biography}</p></section>}
    {releaseGroups.map(({ id, kinds, title }) => {
      const releases = discography.filter((item) => kinds.some((kind) => item.kind === kind));
      if (!releases.length) return null;
      return <DiscographySection key={id} title={title} artist={artist.name} releases={releases} />;
    })}
    {artist.tracks.length > 0 && <section className='artist-releases'>
      <h2>Треки в медиатеке</h2>
      <div className='track-collection' data-layout='list'>
        {artist.tracks.slice(0, trackLimit).map((track) => <Track key={track.id} track={track} onPlay={(track) => play(track, artist.tracks)} />)}
      </div>
      {artist.tracks.length > trackLimit && <Button size='small' onClick={() => setTrackLimit((limit) => limit + 24)}>Показать ещё</Button>}
    </section>}
  </article>;
}
