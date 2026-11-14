import { closePlayerVideo, trackVideoId } from '@/utils/playerVideo';
import { TrackAlbumLink, TrackArtistLinks } from '@/components/TrackLinks';
import { Artwork } from '@/components/MediaCard';
import { type Track } from '@/types/Track';
import { useTrackAudioInfo } from '@/hooks/useTrackAudioInfo';
import { useTrackCover } from '@/hooks/useTrackCover';

export function TrackPlaying({ track }: { track: Track | null }) {
  const { coverSrc, clearCover } = useTrackCover(track?.path);
  const audioInfo = useTrackAudioInfo(track?.path);
  if (!track) return null;

  return (
    <div className='player-track' data-track-path={track.path} data-external-video={trackVideoId(track)} data-local-video={track?.hasLocalVideo || undefined}
      onClick={(event) => { if (event.target instanceof Element && event.target.closest('a')) closePlayerVideo(); }}>
      <TrackAlbumLink track={track}>
        <Artwork className='player-cover' icon='Track' src={coverSrc} onError={clearCover} />
      </TrackAlbumLink>
      <span className='track-label'>
        <span className='media-title' title={track.name}><TrackAlbumLink track={track}>{track.name}</TrackAlbumLink></span>
        <span className='track-artist'><TrackArtistLinks artists={track.artists} /></span>
        {audioInfo && <span className='player-audio-info' title={audioInfo}>{audioInfo}</span>}
      </span>
    </div>
  );
}
