import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { MediaCard } from '@/components/MediaCard';
import { usePlayerStore } from '@/stores';
import { inspectQueue } from '@/stores/playerStore';

export function QueuePanel({ close }: { close: () => void }) {
  const generation = usePlayerStore((state) => state.queueGeneration);
  const current = usePlayerStore((state) => state.queueIndex);
  const jump = usePlayerStore((state) => state.jumpToQueueItem);
  const [queue, setQueue] = useState<Awaited<ReturnType<typeof inspectQueue>>>();
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const visible = queue?.generation === generation ? queue : undefined;

  useEffect(() => {
    let active = true;
    setError('');
    setLoading(true);
    void inspectQueue(0).then((value) => {
      if (active) setQueue(value);
    }).catch((reason) => {
      if (active) setError(String(reason));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, [generation]);

  const loadMore = async () => {
    if (!visible || loading) return;
    setLoading(true);
    setError('');
    try {
      const page = await inspectQueue(visible.entries.length);
      if (page.generation === generation) {
        setQueue((previous) => previous === visible ? {
          ...page, entries: [...visible.entries, ...page.entries],
        } : previous);
      }
    } catch (reason) {
      if (usePlayerStore.getState().queueGeneration === generation) setError(String(reason));
    } finally {
      if (usePlayerStore.getState().queueGeneration === generation) setLoading(false);
    }
  };

  return (
    <section className='queue-panel' aria-label='Очередь'
      onKeyDown={(event) => { if (event.key === 'Escape') close(); }}>
      <div className='flex items-center justify-between gap-2'>
        <h2 className='font-semibold'>Очередь</h2>
        <Button variant='ghost' size='icon' icon='X' aria-label='Закрыть очередь' onClick={close} />
      </div>
      {error && <p role='alert' className='kit-error'>{error}</p>}
      {visible?.length === 0 && <p className='text-(--yap-muted)'>Очередь пуста.</p>}
      <ol className='grid gap-1'>
        {visible?.entries.map(({ path, track }, index) => (
          <li key={index}>
            <MediaCard icon='Track' title={track?.name ?? path.split(/[\\/]/).pop() ?? path}
              subtitle={track?.artists.join(', ')} aria-current={index === current ? 'true' : undefined}
              onClick={() => jump(index, visible.generation)} />
          </li>
        ))}
      </ol>
      {visible && visible.entries.length < visible.length && (
        <Button size='small' disabled={loading} onClick={() => void loadMore()}>Показать ещё</Button>
      )}
    </section>
  );
}
