import { trackVideoId } from '@/utils/playerVideo';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import type { RefObject } from 'preact';
import { useEffect, useId, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { MediaCard } from '@/components/MediaCard';
import { TrackAlbumLink, TrackArtistLinks } from '@/components/TrackLinks';
import { subscribeToCatalog, tracksByPaths } from '@/hooks/useLibrary';
import { useOutsidePointer } from '@/hooks/useOutsidePointer';
import { useIntersection } from '@/hooks/useIntersection';
import { useTrackCover } from '@/hooks/useTrackCover';
import { usePlayerStore } from '@/stores';
import { inspectQueue } from '@/stores/playerStore';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';
import type { Track } from '@/types/Track';

type Queue = Awaited<ReturnType<typeof inspectQueue>>;
const tabs = [
  { id: 'queue', label: 'Очередь', icon: 'Queue' },
  { id: 'history', label: 'История', icon: 'Time' },
] as const;
type QueueTab = (typeof tabs)[number]['id'];

function QueueRow({ path, track, selected, play }: Queue['entries'][number] & {
  selected: boolean;
  play: () => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  const visible = useIntersection(ref, '300px');
  const { coverSrc, clearCover } = useTrackCover(visible ? path : undefined);
  return <MediaCard buttonRef={ref} data-track-path={path} data-external-video={trackVideoId(track)} data-local-video={track?.hasLocalVideo || undefined} icon='Track' coverSrc={coverSrc} onCoverError={clearCover}
    title={track?.name ?? path.split(/[\\/]/).pop() ?? path} subtitle={track?.artists.join(', ')}
    titleLink={track && <TrackAlbumLink track={track}>{track.name}</TrackAlbumLink>}
    subtitleLinks={track?.artists.length ? <TrackArtistLinks artists={track.artists} /> : undefined}
    playable
    metadata={track?.duration ? formatPlaybackTime(track.duration) : undefined}
    aria-current={selected ? 'true' : undefined} onClick={play} />;
}

export function QueuePanel({ close, trigger }: { close: () => void; trigger: RefObject<HTMLButtonElement> }) {
  const panel = useRef<HTMLElement>(null);
  useOutsidePointer(panel, true, close, trigger);
  const generation = usePlayerStore((state) => state.queueGeneration);
  const current = usePlayerStore((state) => state.queueIndex);
  const queueView = useRef<HTMLDivElement>(null);
  const endOfQueue = useRef<HTMLDivElement>(null);
  const revealedGeneration = useRef<number>();
  const jump = usePlayerStore((state) => state.jumpToQueueItem);
  const [queue, setQueue] = useState<Queue>();
  const tabsId = useId();
  const [tab, setTab] = useState<QueueTab>('queue');
  const [historyVisited, setHistoryVisited] = useState(false);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [paging, setPaging] = useState(false);
  const visible = queue?.generation === generation ? queue : undefined;

  const selectTab = (next: QueueTab) => {
    setTab(next);
    if (next === 'history') setHistoryVisited(true);
  };

  const dismiss = () => {
    close();
    trigger.current?.focus();
  };

  useEffect(() => {
    if (visible && (visible.length === 0 || (current >= visible.offset && current < visible.offset + visible.entries.length))) {
      setLoading(false);
      return;
    }
    let active = true;
    setError('');
    setPaging(false);
    setLoading(true);
    void inspectQueue(current).then((page) => {
      if (active) setQueue(page);
    }).catch((reason) => {
      if (active) setError(String(reason));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, [generation, current]);

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
    if (tab !== 'queue' || !visible || revealedGeneration.current === visible.generation) return;
    revealedGeneration.current = visible.generation;
    queueView.current?.scrollTo({ top: 0 });
  }, [tab, visible?.generation]);

  const loadMore = async () => {
    if (!visible || loading || paging) return;
    const offset = visible.offset + visible.entries.length;
    setPaging(true);
    setError('');
    try {
      const page = await inspectQueue(offset);
      if (page.generation === generation) {
        setQueue((value) => value === visible ? {
          ...page, offset: visible.offset,
          entries: [...visible.entries, ...page.entries],
        } : value);
      }
    } catch (reason) {
      if (usePlayerStore.getState().queueGeneration === generation) setError(String(reason));
    } finally {
      if (usePlayerStore.getState().queueGeneration === generation) setPaging(false);
    }
  };

  const hasNextPage = !!visible && visible.offset + visible.entries.length < visible.length;
  useEffect(() => {
    const sentinel = endOfQueue.current;
    if (tab !== 'queue' || !sentinel || !hasNextPage || loading || paging || error) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry?.isIntersecting) {
        observer.disconnect();
        void loadMore();
      }
    }, { root: queueView.current, rootMargin: '100px' });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [visible, hasNextPage, loading, paging, error, tab]);

  return (
    <section ref={panel} id='playback-queue' className='queue-panel' aria-label='Очередь и история воспроизведения'
      onKeyDown={(event) => {
        if (event.key === 'Escape') dismiss();
      }}>
      <div className='queue-panel-header'>
        <div className='queue-panel-tabs' role='tablist' aria-label='Режим просмотра'
          onKeyDown={(event) => {
            const next = event.key === 'Home' ? 'queue' : event.key === 'End' ? 'history'
              : event.key === 'ArrowLeft' || event.key === 'ArrowRight' ? (tab === 'queue' ? 'history' : 'queue') : undefined;
            if (!next) return;
            event.preventDefault();
            selectTab(next);
            document.getElementById(`${tabsId}-tab-${next}`)?.focus();
          }}>
          {tabs.map(({ id, label, icon }) => <Button key={id} size='small' icon={icon} role='tab'
            id={`${tabsId}-tab-${id}`} aria-controls={`${tabsId}-view-${id}`} aria-selected={id === tab}
            tabIndex={id === tab ? 0 : -1} variant={id === tab ? 'primary' : 'ghost'} onClick={() => selectTab(id)}>
            {label}
          </Button>)}
        </div>
        <Button variant='ghost' size='icon' icon='X' aria-label='Закрыть панель' onClick={dismiss} />
      </div>
      <div ref={queueView} className='queue-panel-view' id={`${tabsId}-view-queue`} role='tabpanel'
        aria-labelledby={`${tabsId}-tab-queue`} hidden={tab !== 'queue'} tabIndex={0}>
        {error && <p role='alert' className='kit-error'>{error}</p>}
        {loading && <LoadingIndicator label='Загрузка очереди' />}
        {visible?.length === 0 && <p className='text-(--yap-muted)'>Очередь пуста.</p>}
        {visible && <ol className='grid gap-1' start={Math.max(visible.offset, current) + 1}>
          {visible.entries.map((entry, row) => {
            const index = visible.offset + row;
            return index >= current && <li key={`${index}:${entry.path}`}>
              <QueueRow {...entry} selected={index === current} play={() => jump(index, visible.generation)} />
            </li>;
          })}
        </ol>}
        {hasNextPage && <div ref={endOfQueue}>
          {paging && <LoadingIndicator label='Загрузка следующих треков' />}
          {error && <Button size='small' disabled={loading || paging} onClick={() => void loadMore()}>Показать следующие</Button>}
        </div>}
      </div>
      <div className='queue-panel-view' id={`${tabsId}-view-history`} role='tabpanel'
        aria-labelledby={`${tabsId}-tab-history`} hidden={tab !== 'history'} tabIndex={0}>
        {historyVisited && <PlaybackHistory />}
      </div>
    </section>
  );
}

type HistoryEntry = { id: number; playedAt: number; track: Track };
const historyDate = new Intl.DateTimeFormat('ru', { dateStyle: 'short', timeStyle: 'short' });
const historyPage = (before?: number) => invoke<HistoryEntry[]>('library_playback_history', { before });

function PlaybackHistory() {
  const [entries, setEntries] = useState<HistoryEntry[]>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  useEffect(() => {
    let active = true;
    const refresh = () => void historyPage().then((page) => {
      if (!active) return;
      setEntries((previous) => {
        const oldest = page[page.length - 1]?.id;
        return [...page, ...(oldest === undefined ? [] : previous?.filter(({ id }) => id < oldest) ?? [])];
      });
      if (page.length === 80) setHasMore(true);
      setError(undefined);
    }).catch((reason) => { if (active) setError(String(reason)); });
    const listener = listen('library:playback-history', refresh);
    void listener.then(refresh, refresh);
    return () => {
      active = false;
      void listener.then((unlisten) => unlisten(), () => undefined);
    };
  }, []);

  const loadMore = async () => {
    const before = entries?.[entries.length - 1]?.id;
    if (before === undefined || loading) return;
    setLoading(true);
    setError(undefined);
    try {
      const page = await historyPage(before);
      setHasMore(page.length === 80);
      setEntries((previous) => [
        ...(previous ?? []),
        ...page.filter(({ id }) => id < (previous?.[previous.length - 1]?.id ?? Infinity)),
      ]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  };
  return <>
    {error && <p role='alert' className='kit-error'>{error}</p>}
    {entries === undefined && !error && <LoadingIndicator label='Загрузка истории' />}
    {entries?.length === 0 && <p className='text-(--yap-muted)'>История пока пуста.</p>}
    <ol className='grid gap-2'>
      {entries?.map(({ id, playedAt, track }) => <li key={id}>
        <p className='text-xs text-(--yap-muted)'>{historyDate.format(playedAt * 1000)}</p>
        <QueueRow path={track.path} track={track} selected={false}
          play={() => usePlayerStore.getState().playTrack(track, [track])} />
      </li>)}
    </ol>
    {hasMore && <Button size='small' pending={loading} onClick={() => void loadMore()}>Показать ещё</Button>}
  </>;
}
