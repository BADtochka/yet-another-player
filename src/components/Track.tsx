import { useRef } from 'preact/hooks';
import { MediaCard } from '@/components/MediaCard';
import { usePlayerStore } from '@/stores';
import { useIntersection } from '@/hooks/useIntersection';
import { useTrackCover } from '@/hooks/useTrackCover';
import { type Track as TrackType } from '@/types/Track';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';

export function Track({ track, onPlay, disabled }: {
  track: TrackType;
  onPlay: (track: TrackType) => void;
  disabled?: boolean;
}) {
  const selected = usePlayerStore((state) => state.currentTrackPath === track.path);
  const ref = useRef<HTMLButtonElement>(null);
  const visible = useIntersection(ref, '300px');
  const { coverSrc, clearCover } = useTrackCover(visible ? track.path : undefined);

  return (
    <MediaCard buttonRef={ref} title={track.name} subtitle={track.artists.join(', ') || 'Неизвестный исполнитель'}
      metadata={track.duration && track.duration > 0 && Number.isFinite(track.duration) ? formatPlaybackTime(track.duration) : undefined}
      icon='Track' coverSrc={coverSrc} onCoverError={clearCover} aria-current={selected ? 'true' : undefined}
      disabled={disabled} onClick={() => onPlay(track)} />
  );
}
