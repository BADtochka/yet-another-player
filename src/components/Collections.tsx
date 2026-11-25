import { LoadingIndicator } from '@/components/LoadingIndicator';
import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { ArtistProfilePage } from '@/components/ArtistProfilePage';
import { mergeReleaseTracks, ReleaseTrackCard, releaseArtistNames, releaseTitleKey, titleKey } from '@/components/ReleaseTracks';
import { Button } from '@/components/Button';
import { MediaCard } from '@/components/MediaCard';
import { MediaDetails } from '@/components/MediaDetails';
import { Track } from '@/components/Track';
import { TrackArtistLinks } from '@/components/TrackLinks';
import { useIntersection } from '@/hooks/useIntersection';
import { useAlbumArtwork } from '@/hooks/useAlbumArtwork';
import { useCatalogCover, useTrackCover } from '@/hooks/useTrackCover';
import { usePlayerStore } from '@/stores';
import { useSettingsStore } from '@/stores/settingsStore';
import type { Track as TrackType } from '@/types/Track';
import { type LibrarySort, useTrackViewStore } from '@/stores/trackViewStore';
import { type Collection, type CollectionSummary, catalogNames, formatTrackCount } from '@/utils/collections';
import { useCollections, useCollectionTracks, type CollectionKind } from '@/hooks/useLibrary';
import { useArtistAvatar, useReleaseDetails, useTrackRelease, type AlbumRelease } from '@/hooks/useRemoteCatalog';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';
import { describePlayback } from '@/utils/trackMetadata';

const icons = { album: 'Album', artist: 'User', playlist: 'Playlist' } as const;
const PAGE_SIZE = 48;

export function CollectionCard({ item, kind, open }: { item: CollectionSummary; kind: CollectionKind; open: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const visible = useIntersection(ref, '300px');
  const avatar = useArtistAvatar(visible && kind === 'artist' ? item.name : undefined);
  const portrait = useCatalogCover(avatar.data ?? undefined, undefined, true);
  const local = useTrackCover(visible && !portrait.coverSrc ? item.coverPath ?? undefined : undefined);
  const count = formatTrackCount(item.trackCount);
  const metadata = {
    album: [item.year, count].filter(Boolean).join(' · '),
    artist: 'Исполнитель',
    playlist: item.duration > 0 ? formatPlaybackTime(item.duration) : undefined,
  }[kind];

  return (
    <MediaCard buttonRef={ref} kind={kind} title={item.name} subtitle={kind === 'album' ? item.subtitle : count}
      subtitleLinks={kind === 'album' && item.subtitle
        ? <TrackArtistLinks artists={item.artists ?? [item.subtitle]} allowExternal /> : undefined}
      metadata={metadata} icon={icons[kind]}
      coverSrc={portrait.coverSrc ?? local.coverSrc}
      coverPending={kind === 'artist' ? avatar.loading || portrait.loading || local.loading : undefined}
      onCoverError={portrait.coverSrc ? portrait.clearCover : local.clearCover} onClick={open} />
  );
}

function sortedCollections(items: CollectionSummary[], kind: CollectionKind, sort: LibrarySort) {
  return [...items].sort((a, b) => {
    const byName = catalogNames.compare(a.name, b.name);
    if (sort === 'added' && kind === 'album') {
      return (b.addedAt ?? 0) - (a.addedAt ?? 0) || (b.firstTrackId ?? 0) - (a.firstTrackId ?? 0) || byName;
    }
    if (sort === 'releaseYear' && kind === 'album') return (b.year ?? 0) - (a.year ?? 0) || byName;
    if (sort === 'created' && kind === 'playlist') return catalogNames.compare(b.id, a.id);
    if (sort === 'artist' && kind === 'album') return catalogNames.compare(a.subtitle ?? '', b.subtitle ?? '') || byName;
    if (sort === 'duration' && kind !== 'artist') return a.duration - b.duration || byName;
    return byName;
  });
}

function MoreItems({ advance }: { advance: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const visible = useIntersection(ref, '500px');
  useEffect(() => { if (visible) advance(); }, [visible, advance]);
  return <div ref={ref} className='col-span-full h-4' aria-hidden='true' />;
}

export function CollectionList({ items, kind, empty, open, search = false }: {
  items: CollectionSummary[];
  kind: CollectionKind;
  empty: string;
  open: (id: string) => void;
  search?: boolean;
}) {
  const { layout, sort } = useTrackViewStore();
  const [limit, setLimit] = useState(PAGE_SIZE);
  const sorted = useMemo(() => sortedCollections(items, kind, search ? 'name' : sort), [items, kind, sort, search]);
  if (!items.length) return empty && <p className='kit-message'>{empty}</p>;

  return (
    <div className='track-collection' data-layout={search ? 'grid' : layout}>
      {sorted.slice(0, limit).map((item) => (
        <CollectionCard key={item.id} item={item} kind={kind} open={() => open(item.id)} />
      ))}
      {limit < items.length && <MoreItems key={limit} advance={() => setLimit(limit + PAGE_SIZE)} />}
    </div>
  );
}

export function CatalogCollectionPage({ kind, id, open }: {
  kind: 'album' | 'artist';
  id?: string;
  open: (id?: string) => void;
}) {
  const collections = useCollections(kind);
  const detail = useCollectionTracks(kind, id);
  const error = collections?.error ?? detail?.error;
  if (error) return <p className='kit-message' role='alert'>{error}</p>;
  if (!collections?.data || (id !== undefined && !detail?.data)) {
    return <LoadingIndicator className='kit-message' label='Загрузка медиатеки' />;
  }
  const selected = collections.data.find((item) => item.id === id);
  return selected && detail?.data
    ? kind === 'artist'
      ? <ArtistProfilePage key={selected.id} artist={{ ...selected, tracks: detail.data }} />
      : <CollectionDetails key={selected.id} item={{ ...selected, tracks: detail.data }} kind={kind} />
    : <CollectionList items={kind === 'album' ? collections.data.filter((item) => !item.shortRelease) : collections.data} kind={kind}
      empty={kind === 'album' ? 'В медиатеке пока нет альбомов.' : 'В медиатеке пока нет исполнителей.'} open={open} />;
}

export function ExternalAlbumPage({ release, collection }: { release: AlbumRelease; collection?: string }) {
  const album = useCollectionTracks('album', collection);
  const artist = useCollectionTracks('artist', release.artistId);
  const tracks = new Map((album?.data ?? []).map((track) => [track.id, track]));
  for (const track of artist?.data ?? []) {
    const title = track.album?.trim() || track.name;
    const year = track.releaseDate?.slice(0, 4) ?? (track.year ? String(track.year) : undefined);
    if (!tracks.has(track.id) && releaseTitleKey(title) === releaseTitleKey(release.title)
      && (!year || !release.date || year === release.date.slice(0, 4))
      && (!track.isSingle || release.kind === 'single')
      && ![...tracks.values()].some((candidate) => candidate.albumId !== track.albumId
        && releaseTitleKey(candidate.name) === releaseTitleKey(track.name)
        && (candidate.discNumber ?? 1) === (track.discNumber ?? 1)
        && (!candidate.trackNumber || !track.trackNumber || candidate.trackNumber === track.trackNumber))) {
      tracks.set(track.id, track);
    }
  }
  if ((collection && !album) || (release.artistId && !artist)) {
    return <LoadingIndicator className='kit-message' label='Загрузка альбома' />;
  }
  return <CollectionDetails kind='album' item={{ id: collection ?? '', name: release.title,
    subtitle: release.artist, tracks: [...tracks.values()] }}
    albumRelease={release} knownArtistTracks={artist?.data} />;
}

export function CollectionDetails({ item, kind, back, actions, trackAction, onPlay, albumRelease, knownArtistTracks }: {
  item: Collection;
  kind: CollectionKind;
  back?: () => void;
  actions?: ComponentChildren;
  trackAction?: (id: number) => ComponentChildren;
  onPlay?: (track: TrackType) => void;
  albumRelease?: AlbumRelease;
  knownArtistTracks?: TrackType[];
}) {
  const [limit, setLimit] = useState(PAGE_SIZE);
  const playTrack = usePlayerStore((state) => state.playTrack);
  const first = item.tracks[0];
  const icon = icons[kind];
  const showExternalTracks = useSettingsStore((state) => state.showExternalAlbumTracks && state.metadataSource !== 'off');
  const external = useReleaseDetails(showExternalTracks ? albumRelease : undefined);
  const useFallback = !!first && (!albumRelease || external.data === null || !!external.error);
  const discovered = useTrackRelease(kind === 'album' && showExternalTracks && useFallback ? first : undefined);
  const lookup = albumRelease && !useFallback ? external : discovered;
  const release = lookup.data;
  const rows = useMemo(() => mergeReleaseTracks(item.tracks, release), [item.tracks, release]);
  const playable = useMemo(() => rows.flatMap((row) => 'local' in row ? [row.local] : []), [rows]);
  const start = onPlay ?? ((track: TrackType) => playTrack(track, playable));
  const linkedArtists = new Map<string, string>();
  for (const track of [...item.tracks, ...(knownArtistTracks ?? [])]) {
    for (const artist of track.artists) linkedArtists.set(titleKey(artist), artist);
  }
  const credit = (albumRelease ? release?.artist || item.subtitle || '' : item.subtitle || release?.artist || '').trim();
  const { src: artwork, onError: clearArtwork, loading: artworkPending } = useAlbumArtwork({
    visible: kind === 'album', track: first, albumId: item.id || undefined,
    release: albumRelease, artwork: release?.artwork, waitingForDetails: lookup.loading,
    artist: first?.albumArtist?.trim() || credit || first?.artists.join(', '), title: item.name,
    year: first?.releaseDate?.slice(0, 4) ?? (first?.year ? String(first.year) : undefined),
  });
  const albumCover = kind === 'album' ? { src: artwork, onError: clearArtwork } : undefined;
  const single = albumRelease ? albumRelease.kind === 'single' : kind === 'album' && (item.id.startsWith('single:')
    || (!item.tracks.some((track) => track.isSingle === false)
      && (item.tracks.some((track) => track.isSingle === true) || / - single$/iu.test(item.name.trim()))));
  const subtitle = kind === 'album' && credit
    ? <TrackArtistLinks artists={releaseArtistNames(credit, linkedArtists)} allowExternal /> : item.subtitle;

  return (
    <section className='collection-details'>
      <MediaDetails item={{ name: single ? first?.name ?? item.name : item.name, subtitle, icon, artwork }}
        onArtworkError={clearArtwork}
        artworkPending={kind === 'album' ? artworkPending : undefined}
        contextTrack={single ? first : undefined}
        onCoverPlay={single && first ? () => start(first) : undefined}
        metadata={[
          ...(kind === 'album' ? describePlayback(item.tracks, release ?? albumRelease) : []),
          ...(single || (!rows.length && albumRelease) ? [] : [{ icon: 'Hash' as const, label: formatTrackCount(rows.length) }]),
        ]}>
        {(back || actions) && <>
          {back && <>
            <Button size='small' onClick={back}>Назад</Button>
            {first && <Button variant='primary' size='small' icon='PlayV2' onClick={() => start(first)}>Воспроизвести</Button>}
          </>}
          {actions}
        </>}
      </MediaDetails>
      <div className='collection-track-panel'>
        <div className='track-collection' data-layout='list' data-release-actions={rows.some((row) => 'remote' in row) || undefined}>
          {rows.slice(0, limit).map((row) => (
            <div key={row.key} className='collection-track'>
              {'local' in row ? <>
                <Track track={row.local} albumCover={albumCover} onPlay={start} />
                {trackAction?.(row.local.id)}
              </> : <ReleaseTrackCard track={row.remote} album={release?.title ?? item.name}
                artist={credit} artwork={artwork} onArtworkError={clearArtwork} linkedArtists={linkedArtists} />}
            </div>
          ))}
          {limit < rows.length && <MoreItems key={limit} advance={() => setLimit(limit + PAGE_SIZE)} />}
        </div>
        {lookup.loading && <LoadingIndicator className='kit-message' label='Загрузка треклиста' />}
        {lookup.error && <div className='release-error'><p role='alert'>Не удалось загрузить треклист.</p>
          <Button size='small' onClick={lookup.retry}>Повторить</Button></div>}
        {albumRelease && !rows.length && !lookup.loading && !lookup.error && <p className='kit-message'>
          {showExternalTracks ? 'Треклист недоступен.' : 'В медиатеке нет треков этого альбома.'}
        </p>}
        {release && !release.complete && <p className='release-muted'>Источник вернул неполный треклист.</p>}
      </div>
    </section>
  );
}
