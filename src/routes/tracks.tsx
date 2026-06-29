import { createFileRoute } from '@tanstack/react-router';
import { load } from '@tauri-apps/plugin-store';
import { useAtom } from 'jotai';
import { useEffect, useState } from 'preact/hooks';
import { tracksAtom } from '../atoms/music';
import { Track } from '../components/Track';
import { type Track as TrackType } from '../types/Track';

const Tracks = () => {
  const [tracks, setTracks] = useAtom(tracksAtom);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const onTracksLoad = async () => {
    try {
      const musicStore = await load('music.json', {
        defaults: {
          tracks: [],
        },
      });

      setTracks((await musicStore.get<TrackType[]>('tracks')) ?? []);
    } catch (loadError) {
      setError(loadError instanceof Error ? loadError.message : String(loadError));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    void onTracksLoad();
  }, []);

  if (isLoading) {
    return <p className='p-4 text-white'>Загрузка треков...</p>;
  }

  if (error) {
    return <p className='p-4 text-[#da3e3e]'>{error}</p>;
  }

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
