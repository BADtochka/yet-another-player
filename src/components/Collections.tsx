import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { Button } from '@/components/Button';
import { MediaCard } from '@/components/MediaCard';
import { MediaDetails } from '@/components/MediaDetails';
import { Track } from '@/components/Track';
import { useIntersection } from '@/hooks/useIntersection';
import { useTrackCover } from '@/hooks/useTrackCover';
import { usePlayerStore } from '@/stores';
import { type LibrarySort, useTrackViewStore } from '@/stores/trackViewStore';
import { type Collection, type CollectionSummary, catalogNames, formatTrackCount } from '@/utils/collections';
import { useCollections, useCollectionTracks, type CollectionKind } from '@/hooks/useLibrary';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';

const icons = { album: 'Album', artist: 'User', playlist: 'Playlist' } as const;
const PAGE_SIZE = 48;

export function CollectionCard({ item, kind, open }: { item: CollectionSummary; kind: CollectionKind; open: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const visible = useIntersection(ref, '300px');
  const { coverSrc, clearCover } = useTrackCover(visible ? item.coverPath ?? undefined : undefined);
  const count = formatTrackCount(item.trackCount);
  const metadata = {
    album: [item.year, count].filter(Boolean).join(' · '),
    artist: 'Исполнитель',
    playlist: item.duration > 0 ? formatPlaybackTime(item.duration) : undefined,
  }[kind];

  return (
    <MediaCard buttonRef={ref} kind={kind} title={item.name} subtitle={kind === 'album' ? item.subtitle : count}
      metadata={metadata} icon={icons[kind]} coverSrc={coverSrc} onCoverError={clearCover} onClick={open} />
  );
}

function sortedCollections(items: CollectionSummary[], kind: CollectionKind, sort: LibrarySort) {
  return [...items].sort((a, b) => {
    const byName = catalogNames.compare(a.name, b.name);
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
  const sort = useTrackViewStore((state) => state.sort);
  const error = collections?.error ?? detail?.error;
  if (error) return <p className='kit-message' role='alert'>{error}</p>;
  if (!collections?.data || (id !== undefined && !detail?.data)) {
    return <p className='kit-message' role='status'>Загружаем медиатеку…</p>;
  }
  const selected = collections.data.find((item) => item.id === id);
  const sorted = kind === 'album' && selected ? sortedCollections(collections.data, kind, sort) : [];
  const next = sorted[sorted.findIndex((item) => item.id === id) + 1];
  return selected && detail?.data
    ? <CollectionDetails key={selected.id} item={{ ...selected, tracks: detail.data }} kind={kind} back={() => open()}
      next={next} onNext={next && (() => open(next.id))} />
    : <CollectionList items={collections.data} kind={kind}
      empty={kind === 'album' ? 'В медиатеке пока нет альбомов.' : 'В медиатеке пока нет исполнителей.'} open={open} />;
}

export function CollectionDetails({ item, kind, back, actions, trackAction, next, onNext }: {
  item: Collection;
  kind: CollectionKind;
  back: () => void;
  actions?: ComponentChildren;
  trackAction?: (id: number) => ComponentChildren;
  next?: CollectionSummary;
  onNext?: () => void;
}) {
  const [limit, setLimit] = useState(PAGE_SIZE);
  const playTrack = usePlayerStore((state) => state.playTrack);
  const first = item.tracks[0];
  const icon = icons[kind];
  const year = kind === 'album' ? item.tracks.find((track) => track.year)?.year : undefined;
  const genre = kind === 'album' ? item.tracks.find((track) => track.genre)?.genre : undefined;

  return (
    <section className='collection-details'>
      <MediaDetails item={{ name: item.name, subtitle: item.subtitle, icon, path: kind === 'album' ? first?.path : undefined }}
        metadata={[
          ...(year ? [{ icon: 'Calendar' as const, label: String(year) }] : []),
          ...(genre ? [{ icon: 'Melody' as const, label: genre }] : []),
          { icon: 'Hash', label: formatTrackCount(item.tracks.length) },
        ]}
        next={next && { name: next.name, subtitle: next.subtitle, icon, path: next.coverPath }} onNext={onNext}>
        <Button size='small' onClick={back}>Назад</Button>
        {first && <Button variant='primary' size='small' icon='PlayV2' onClick={() => playTrack(first, item.tracks)}>Воспроизвести</Button>}
        {actions}
      </MediaDetails>
      <div className='track-collection' data-layout='list'>
        {item.tracks.slice(0, limit).map((track) => (
          <div key={track.id} className='collection-track'>
            <Track track={track} onPlay={(selected) => playTrack(selected, item.tracks)} />
            {trackAction?.(track.id)}
          </div>
        ))}
        {limit < item.tracks.length && <MoreItems key={limit} advance={() => setLimit(limit + PAGE_SIZE)} />}
      </div>
    </section>
  );
}
