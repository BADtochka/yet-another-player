import { createFileRoute } from '@tanstack/react-router';
import { Track } from '@/components/Track';
import { useMusicStore } from '@/stores';

const Tracks = () => {
  const tracks = useMusicStore((state) => state.tracks);

  if (tracks.length === 0) {
    return <p className='p-4 text-white'>MP3 файлы не найдены.</p>;
  }

  return (
    <div className='grid min-h-0 w-full content-start grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3'>
      {tracks.map((track) => (
        <Track key={track.path} track={track} />
      ))}
    </div>
  );
};

export const Route = createFileRoute('/tracks')({
  component: Tracks,
});
