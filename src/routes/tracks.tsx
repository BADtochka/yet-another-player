import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Track } from '@/components/Track';
import { useMusicStore } from '@/stores';

const TRACKS_PAGE_SIZE = 48;

const Tracks = () => {
  const tracks = useMusicStore((state) => state.tracks);
  const [visibleCount, setVisibleCount] = useState(TRACKS_PAGE_SIZE);
  const sentinelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setVisibleCount(TRACKS_PAGE_SIZE);
  }, [tracks.length]);

  useEffect(() => {
    const sentinel = sentinelRef.current;

    if (!sentinel || visibleCount >= tracks.length) {
      return;
    }

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) {
          setVisibleCount((current) => Math.min(current + TRACKS_PAGE_SIZE, tracks.length));
        }
      },
      { rootMargin: '600px' },
    );

    observer.observe(sentinel);

    return () => {
      observer.disconnect();
    };
  }, [tracks.length, visibleCount]);

  if (tracks.length === 0) {
    return <p className='p-4 text-white'>MP3 файлы не найдены.</p>;
  }

  const visibleTracks = tracks.slice(0, visibleCount);

  return (
    <div className='grid min-h-0 w-full content-start grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3'>
      {visibleTracks.map((track) => (
        <Track key={track.path} track={track} />
      ))}
      {visibleCount < tracks.length && <div ref={sentinelRef} className='col-span-full h-8' aria-hidden='true' />}
    </div>
  );
};

export const Route = createFileRoute('/tracks')({
  component: Tracks,
});
