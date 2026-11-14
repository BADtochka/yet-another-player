import { trackVideoId } from '@/utils/playerVideo';
import { TrackAlbumLink, TrackArtistLinks } from '@/components/TrackLinks';
import type { Track } from '@/types/Track';
import type { ComponentChildren } from 'preact';
import { Button } from '@/components/Button';
import { Artwork } from '@/components/MediaCard';
import { Icon, type IconName } from '@/components/Icons/Icon';
import { useTrackCover } from '@/hooks/useTrackCover';

type Summary = { name: string; subtitle?: string | null; icon: IconName; path?: string | null; track?: Track };

export function MediaDetails({ item, metadata, next, onNext, children }: {
  item: Summary;
  metadata?: Array<{ icon: IconName; label: string }>;
  next?: Summary;
  onNext?: () => void;
  children?: ComponentChildren;
}) {
  const cover = useTrackCover(item.path ?? undefined);
  const art = <Artwork className='collection-art' icon={item.icon} src={cover.coverSrc} onError={cover.clearCover} />;
  return (
    <aside className='collection-summary' data-track-path={item.track?.path} data-external-video={trackVideoId(item.track)} data-local-video={item.track?.hasLocalVideo || undefined}>
      {item.track ? <TrackAlbumLink track={item.track}>{art}</TrackAlbumLink> : art}
      <header>
        <h1>{item.track ? <TrackAlbumLink track={item.track}>{item.name}</TrackAlbumLink> : item.name}</h1>
        {item.track ? <p><TrackArtistLinks artists={item.track.artists} /></p> : item.subtitle && <p>{item.subtitle}</p>}
      </header>
      {metadata && metadata.length > 0 && (
        <ul className='media-details-meta'>
          {metadata.map(({ icon, label }) => <li key={icon}><Icon name={icon} /><span>{label}</span></li>)}
        </ul>
      )}
      {next && <NextMedia item={next} onClick={onNext} />}
      {children && <div className='media-details-actions'>{children}</div>}
    </aside>
  );
}

function NextMedia({ item, onClick }: { item: Summary; onClick?: () => void }) {
  const cover = useTrackCover(item.path ?? undefined);
  const art = <Artwork icon={item.icon} src={cover.coverSrc} onError={cover.clearCover} />;
  const content = (
    <>
      {item.track ? <TrackAlbumLink track={item.track}>{art}</TrackAlbumLink> : art}
      <span className='track-label'>
        <span className='media-title'>{item.track ? <TrackAlbumLink track={item.track}>{item.name}</TrackAlbumLink> : item.name}</span>
        {item.track ? <span className='track-artist'><TrackArtistLinks artists={item.track.artists} /></span>
          : item.subtitle && <span className='track-artist'>{item.subtitle}</span>}
      </span>
    </>
  );
  return (
    <div className='media-details-next'>
      <p>Дальше</p>
      {onClick ? <Button variant='ghost' className='media-details-next-card' onClick={onClick}>{content}</Button>
        : <div className='media-details-next-card' data-track-path={item.track?.path} data-external-video={trackVideoId(item.track)} data-local-video={item.track?.hasLocalVideo || undefined}>{content}</div>}
    </div>
  );
}
