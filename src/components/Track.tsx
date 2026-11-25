import { trackVideoId } from '@/utils/playerVideo';
import { useRef } from 'preact/hooks';
import { MediaCard } from '@/components/MediaCard';
import { TrackAlbumLink, TrackArtistLinks } from '@/components/TrackLinks';
import { usePlayerStore } from '@/stores';
import { useIntersection } from '@/hooks/useIntersection';
import { useTrackCover } from '@/hooks/useTrackCover';
import { type Track as TrackType } from '@/types/Track';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';

export function Track({ track, onPlay, disabled, selectionOnly = false, albumCover }: {
  track: TrackType;
  onPlay: (track: TrackType) => void;
  disabled?: boolean;
  selectionOnly?: boolean;
  albumCover?: { src?: string | null; onError: () => void };
}) {
  const selected = usePlayerStore((state) => state.currentTrackPath === track.path);
  const ref = useRef<HTMLButtonElement>(null);
  const visible = useIntersection(ref, '300px');
  const { coverSrc, clearCover } = useTrackCover(visible && !albumCover ? track.path : undefined);

  return (
    <MediaCard data-track-path={track.path} data-external-video={trackVideoId(track)} data-local-video={track?.hasLocalVideo || undefined} buttonRef={ref} title={track.name} subtitle={track.artists.join(', ') || 'Неизвестный исполнитель'}
      titleLink={selectionOnly ? undefined : <TrackAlbumLink track={track}>{track.name}</TrackAlbumLink>}
      subtitleLinks={track.artists.length > 0 ? <TrackArtistLinks artists={track.artists} /> : undefined}
      playable={!selectionOnly}
      metadata={track.duration && track.duration > 0 && Number.isFinite(track.duration) ? formatPlaybackTime(track.duration) : undefined}
      icon='Track' coverSrc={albumCover ? albumCover.src : coverSrc} onCoverError={albumCover?.onError ?? clearCover}
      aria-current={selected ? 'true' : undefined}
      aria-label={selectionOnly ? `Добавить «${track.name}» в плейлист` : `Воспроизвести «${track.name}»`}
      disabled={disabled} onClick={() => onPlay(track)} />
  );
}
