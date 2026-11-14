import { LoadingIndicator } from '@/components/LoadingIndicator';
import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useRef } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Track } from '@/components/Track';
import { useTrackPages } from '@/hooks/useLibrary';
import { playTrackQuery } from '@/stores/playerStore';
import { useTrackViewStore } from '@/stores/trackViewStore';

const Tracks = () => {
  const { layout, sort: preference } = useTrackViewStore();
  const sort = preference === 'releaseYear' || preference === 'created' ? 'name' : preference;
  const query = { sort, text: '' };
  const { tracks, loading, loadingMore, error, hasMore, loadMore, retry } = useTrackPages(query);
  const sentinel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const target = sentinel.current;
    if (!target || error || loading || loadingMore) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry?.isIntersecting) {
        observer.disconnect();
        loadMore();
      }
    }, { rootMargin: '600px' });
    observer.observe(target);
    return () => observer.disconnect();
  }, [tracks, hasMore, error, loading, loadingMore, sort]);

  if (loading && !tracks.length) return <LoadingIndicator className='kit-message' label='Загрузка треков' />;
  if (!tracks.length && !error) return <p className='kit-message'>В медиатеке пока нет треков.</p>;

  return (
    <div className='track-collection' data-layout={layout}>
      {tracks.map((track) => (
        <Track key={track.id} track={track} onPlay={() => playTrackQuery(query, { type: 'id', id: track.id })} />
      ))}
      {error && <div className='col-span-full flex flex-wrap items-center gap-2' role='alert'>
        <span className='kit-error'>{error}</span><Button size='small' onClick={retry}>Повторить</Button>
      </div>}
      {(loading || loadingMore) && <LoadingIndicator className='col-span-full kit-message' label='Загрузка треков' />}
      {hasMore && <div ref={sentinel} className='col-span-full h-8' aria-hidden='true' />}
    </div>
  );
};

export const Route = createFileRoute('/tracks')({ component: Tracks });
