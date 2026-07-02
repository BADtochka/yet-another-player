import { useEffect, useRef, useState } from 'preact/hooks';
import { usePlayerStore } from '@/stores';
import { useIntersection } from '@/hooks/useIntersection';
import { type Track as TrackType } from '@/types/Track';
import { getCoverSrc } from '@/utils/audioServer';

const FALLBACK_COVER = '/temp_cover.png';

type TrackProps = {
  track: TrackType;
};

export const Track = ({ track }: TrackProps) => {
  const setCurrentTrack = usePlayerStore((state) => state.setCurrentTrack);
  const setCurrentTime = usePlayerStore((state) => state.setCurrentTime);
  const setPlaying = usePlayerStore((state) => state.setPlaying);
  const rootRef = useRef<HTMLDivElement>(null);
  const isVisible = useIntersection(rootRef, '300px');
  const [coverSrc, setCoverSrc] = useState(FALLBACK_COVER);
  const { name, artists, album, year, path } = track;
  const artistLabel = artists.length > 0 ? artists.join(', ') : 'Unknown artist';
  const metadata = [album, year].filter(Boolean).join(' · ');

  useEffect(() => {
    if (!isVisible) {
      return;
    }

    let cancelled = false;

    void getCoverSrc(path)
      .then((src) => {
        if (!cancelled) {
          setCoverSrc(src);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setCoverSrc(FALLBACK_COVER);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [isVisible, path]);

  const onTrackClicked = () => {
    requestAnimationFrame(() => {
      setCurrentTrack(track);
      setCurrentTime(0);
      setPlaying(true);
    });
  };

  return (
    <div
      ref={rootRef}
      className='flex flex-col gap-2 rounded-2xl border border-[#101C3A] bg-[#061232] p-3 cursor-pointer'
      onPointerDown={onTrackClicked}
    >
      <img
        src={coverSrc}
        alt=''
        decoding='async'
        loading='lazy'
        className='aspect-square w-full rounded-lg object-cover'
        onError={(event) => {
          const image = event.currentTarget;

          if (!image.src.includes('temp_cover.png')) {
            image.src = FALLBACK_COVER;
          }
        }}
      />
      <div className='flex flex-col gap-1'>
        <p className='line-clamp-1 text-ellipsis font-semibold'>{name}</p>
        <p className='line-clamp-1 overflow-hidden text-ellipsis wrap-break-word'>{artistLabel}</p>
        {metadata && <p className='line-clamp-1 text-ellipsis text-sm text-white/60'>{metadata}</p>}
      </div>
    </div>
  );
};
