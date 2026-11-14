import { trackVideoId } from '@/utils/playerVideo';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import type { RefObject } from 'preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { MediaCard } from '@/components/MediaCard';
import { subscribeToCatalog, tracksByPaths } from '@/hooks/useLibrary';
import { useOutsidePointer } from '@/hooks/useOutsidePointer';
import { useIntersection } from '@/hooks/useIntersection';
import { useTrackCover } from '@/hooks/useTrackCover';
import { usePlayerStore } from '@/stores';
import { inspectQueue } from '@/stores/playerStore';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';

type Queue = Awaited<ReturnType<typeof inspectQueue>>;

function QueueRow({ path, track, selected, play }: Queue['entries'][number] & {
  selected: boolean;
  play: () => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  const visible = useIntersection(ref, '300px');
  const { coverSrc, clearCover } = useTrackCover(visible ? path : undefined);
  return <MediaCard buttonRef={ref} data-track-path={path} data-external-video={trackVideoId(track)} data-local-video={track?.hasLocalVideo || undefined} icon='Track' coverSrc={coverSrc} onCoverError={clearCover}
    title={track?.name ?? path.split(/[\\/]/).pop() ?? path} subtitle={track?.artists.join(', ')}
    metadata={track?.duration ? formatPlaybackTime(track.duration) : undefined}
    aria-current={selected ? 'true' : undefined} onClick={play} />;
}

export function QueuePanel({ close, trigger }: { close: () => void; trigger: RefObject<HTMLButtonElement> }) {
  const panel = useRef<HTMLElement>(null);
  useOutsidePointer(panel, true, close, trigger);
  const generation = usePlayerStore((state) => state.queueGeneration);
  const current = usePlayerStore((state) => state.queueIndex);
  const endOfQueue = useRef<HTMLDivElement>(null);
  const currentRow = useRef<HTMLLIElement>(null);
  const jump = usePlayerStore((state) => state.jumpToQueueItem);
  const [queue, setQueue] = useState<Queue>();
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [paging, setPaging] = useState<'previous' | 'next'>();
  const visible = queue?.generation === generation ? queue : undefined;

  useEffect(() => {
    let active = true;
    setError('');
    setPaging(undefined);
    setLoading(true);
    const index = usePlayerStore.getState().queueIndex;
    void inspectQueue(Math.max(0, index - 8)).then((page) => {
      if (active) setQueue(page);
    }).catch((reason) => {
      if (active) setError(String(reason));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, [generation]);

  useEffect(() => {
    if (!visible || paging) return;
    let active = true;
    const unsubscribe = subscribeToCatalog(() => {
      const paths = [...new Set(visible.entries.map(({ path }) => path))];
      const batches: string[][] = [];
      for (let offset = 0; offset < paths.length; offset += 100) batches.push(paths.slice(offset, offset + 100));
      void Promise.all(batches.map(tracksByPaths)).then((results) => {
        if (!active) return;
        const metadata = new Map(results.flat().map((track) => [track.path, track]));
        setQueue((current) => current === visible ? {
          ...visible, entries: visible.entries.map((entry) => ({ ...entry, track: metadata.get(entry.path) ?? entry.track })),
        } : current);
      }).catch(() => undefined);
    });
    return () => { active = false; unsubscribe(); };
  }, [visible, paging]);

  useEffect(() => {
    if (visible && currentRow.current) currentRow.current.scrollIntoView({ block: 'nearest' });
  }, [visible?.generation]);

  const loadMore = async (previous: boolean) => {
    if (!visible || loading || paging) return;
    const offset = previous ? Math.max(0, visible.offset - 80) : visible.offset + visible.entries.length;
    setPaging(previous ? 'previous' : 'next');
    setError('');
    try {
      const page = await inspectQueue(offset);
      if (page.generation === generation) {
        setQueue((value) => value === visible ? {
          ...page, offset: previous ? offset : visible.offset,
          entries: previous
            ? [...page.entries.slice(0, visible.offset - offset), ...visible.entries]
            : [...visible.entries, ...page.entries],
        } : value);
      }
    } catch (reason) {
      if (usePlayerStore.getState().queueGeneration === generation) setError(String(reason));
    } finally {
      if (usePlayerStore.getState().queueGeneration === generation) setPaging(undefined);
    }
  };

  const hasNextPage = !!visible && visible.offset + visible.entries.length < visible.length;
  useEffect(() => {
    const sentinel = endOfQueue.current;
    if (!sentinel || !hasNextPage || loading || paging || error) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry?.isIntersecting) {
        observer.disconnect();
        void loadMore(false);
      }
    }, { root: panel.current, rootMargin: '100px' });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [visible, hasNextPage, loading, paging, error]);

  return (
    <section ref={panel} id='playback-queue' className='queue-panel' aria-label='Очередь'
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          close();
          trigger.current?.focus();
        }
      }}>
      <div className='flex items-center justify-between gap-2'>
        <h2 className='font-semibold'>Очередь</h2>
        <Button variant='ghost' size='icon' icon='X' aria-label='Закрыть очередь' onClick={() => { close(); trigger.current?.focus(); }} />
      </div>
      {error && <p role='alert' className='kit-error'>{error}</p>}
      {loading && <LoadingIndicator label='Загрузка очереди' />}
      {visible?.length === 0 && <p className='text-(--yap-muted)'>Очередь пуста.</p>}
      {visible && visible.offset > 0 && (
        <Button size='small' disabled={loading || !!paging} pending={paging === 'previous'} onClick={() => void loadMore(true)}>Показать предыдущие</Button>
      )}
      {visible && <ol className='grid gap-1' start={visible.offset + 1}>
        {visible.entries.map((entry, row) => {
          const index = visible.offset + row;
          return <li key={`${index}:${entry.path}`} ref={index === current ? currentRow : undefined}>
            <QueueRow {...entry} selected={index === current} play={() => jump(index, visible.generation)} />
          </li>;
        })}
      </ol>}
      {hasNextPage && <div ref={endOfQueue}>
        {paging === 'next' && <LoadingIndicator label='Загрузка следующих треков' />}
        {error && <Button size='small' disabled={loading || !!paging} onClick={() => void loadMore(false)}>Показать следующие</Button>}
      </div>}
    </section>
  );
}
