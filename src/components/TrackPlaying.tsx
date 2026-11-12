import { Artwork } from '@/components/MediaCard';
import { type Track } from '@/types/Track';
import { useTrackCover } from '@/hooks/useTrackCover';

export function TrackPlaying({ track }: { track: Track | null }) {
  const { coverSrc, clearCover } = useTrackCover(track?.path);
  if (!track) return null;

  return (
    <div className='player-track'>
      <Artwork className='player-cover' icon='Track' src={coverSrc} onError={clearCover} />
      <span className='track-label'>
        <span className='media-title' title={track.name}>{track.name}</span>
        <span className='track-artist'>{track.artists.join(', ') || 'Неизвестный исполнитель'}</span>
      </span>
    </div>
  );
}
