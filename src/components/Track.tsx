import { useSetAtom } from 'jotai';
import { currentTrackAtom, playingAtom } from '../atoms/music';
import { type Track as TrackType } from '../types/Track';

type TrackProps = {
  track: TrackType;
};

export const Track = ({ track }: TrackProps) => {
  const setCurrentTrack = useSetAtom(currentTrackAtom);
  const setPlaying = useSetAtom(playingAtom);
  const { name, artists, coverBase64, album, year } = track;
  const artistLabel = artists.length > 0 ? artists.join(', ') : 'Unknown artist';
  const metadata = [album, year].filter(Boolean).join(' · ');

  const onTrackClicked = () => {
    requestAnimationFrame(() => {
      setCurrentTrack(track);
      setPlaying(true);
    });
  };

  return (
    <div
      className='flex flex-col gap-2 rounded-2xl border border-[#101C3A] bg-[#061232] p-3 cursor-pointer'
      onPointerDown={onTrackClicked}
    >
      <img src={coverBase64 || '/temp_cover.png'} className='rounded-lg aspect-square object-cover' />
      <div className='flex flex-col gap-1'>
        <p className='font-semibold line-clamp-1 text-ellipsis'>{name}</p>
        <p class='wrap-break-word text-ellipsis line-clamp-1 overflow-hidden'>{artistLabel}</p>
        {metadata && <p className='text-sm text-white/60 line-clamp-1 text-ellipsis'>{metadata}</p>}
      </div>
    </div>
  );
};
