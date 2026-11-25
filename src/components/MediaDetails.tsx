import { ScrollingTrackArtists } from '@/components/ScrollingTrackArtists';
import { trackVideoId } from '@/utils/playerVideo';
import { TrackAlbumLink, TrackArtistLinks } from '@/components/TrackLinks';
import type { Track } from '@/types/Track';
import type { ComponentChildren } from 'preact';
import { Artwork } from '@/components/MediaCard';
import { Icon, type IconName } from '@/components/Icons/Icon';
import { useTrackCover } from '@/hooks/useTrackCover';
import { useState } from 'preact/hooks';

type Summary = { name: string; subtitle?: ComponentChildren; icon: IconName; path?: string | null; track?: Track; artwork?: string | null };

export function MediaDetails({ item, metadata, next, onCoverPlay, onArtworkError, artworkPending, contextTrack, children }: {
  item: Summary;
  metadata?: Array<{ icon: IconName; label: string }>;
  next?: Summary;
  onCoverPlay?: () => void;
  onArtworkError?: () => void;
  artworkPending?: boolean;
  contextTrack?: Track;
  children?: ComponentChildren;
}) {
  const track = item.track ?? contextTrack;
  const cover = useTrackCover(item.path ?? undefined);
  const [brokenArtwork, setBrokenArtwork] = useState<string>();
  const artwork = item.artwork && item.artwork !== brokenArtwork ? item.artwork : undefined;
  const art = <Artwork className='collection-art' icon={item.icon} src={cover.coverSrc ?? artwork}
    pending={artworkPending}
    onError={cover.coverSrc ? cover.clearCover : onArtworkError ?? (() => setBrokenArtwork(artwork))}>
    {onCoverPlay && <span className='track-cover-play'><Icon name='PlayV2' /></span>}
  </Artwork>;
  return (
    <aside className='collection-summary' data-track-path={track?.path} data-external-video={trackVideoId(track)} data-local-video={track?.hasLocalVideo || undefined}>
      {onCoverPlay
        ? <button type='button' className='collection-cover-play' aria-label={`Воспроизвести «${item.name}»`}
          onClick={onCoverPlay}>{art}</button>
        : item.track ? <TrackAlbumLink track={item.track}>{art}</TrackAlbumLink> : art}
      <header>
        <h1>{item.track ? <TrackAlbumLink track={item.track}>{item.name}</TrackAlbumLink> : item.name}</h1>
        {item.track ? <p><TrackArtistLinks artists={item.track.artists} /></p> : item.subtitle && <p>{item.subtitle}</p>}
      </header>
      {metadata && metadata.length > 0 && (
        <ul className='media-details-meta'>
          {metadata.map(({ icon, label }) => <li key={icon}><Icon name={icon} /><span>{label}</span></li>)}
        </ul>
      )}
      {next && <NextMedia item={next} />}
      {children && <div className='media-details-actions'>{children}</div>}
    </aside>
  );
}

function NextMedia({ item }: { item: Summary }) {
  const cover = useTrackCover(item.path ?? undefined);
  const art = <Artwork icon={item.icon} src={cover.coverSrc} onError={cover.clearCover} />;
  const content = (
    <>
      {item.track ? <TrackAlbumLink track={item.track}>{art}</TrackAlbumLink> : art}
      <span className='track-label'>
        <span className='media-title'>{item.track ? <TrackAlbumLink track={item.track}>{item.name}</TrackAlbumLink> : item.name}</span>
        {item.track ? <ScrollingTrackArtists key={item.track.path} artists={item.track.artists} />
          : item.subtitle && <span className='track-artist'>{item.subtitle}</span>}
      </span>
    </>
  );
  return (
    <div className='media-details-next'>
      <p>Дальше</p>
      <div className='media-details-next-card' data-track-path={item.track?.path}
        data-external-video={trackVideoId(item.track)} data-local-video={item.track?.hasLocalVideo || undefined}>{content}</div>
    </div>
  );
}
