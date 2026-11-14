import { LoadingIndicator } from '@/components/LoadingIndicator';
import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { Input } from '@/components/Input';
import { Track } from '@/components/Track';
import { CollectionDetails, CollectionList } from '@/components/Collections';
import { useCollections, useCollectionTracks, useTrackPages } from '@/hooks/useLibrary';
import { changePlaylists, usePlaylistStore } from '@/stores/playlistStore';

export const Route = createFileRoute('/playlists')({
  validateSearch: (search: Record<string, unknown>) => ({
    collection: typeof search.collection === 'string' ? search.collection : undefined,
  }),
  component: Playlists,
});

type Editor = 'create' | 'rename' | 'delete' | 'add';

function Playlists() {
  const playlists = usePlaylistStore((state) => state.playlists);
  const [error, setError] = useState('');
  const [editorError, setEditorError] = useState('');
  const [editor, setEditor] = useState<Editor>();
  const [name, setName] = useState('');
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const { collection } = Route.useSearch();
  const navigate = Route.useNavigate();
  const open = (id?: string) => void navigate({ search: { collection: id } });
  const playlist = playlists.find((item) => String(item.id) === collection);
  const summaries = useCollections('playlist');
  const detail = useCollectionTracks('playlist', playlist ? collection : undefined);
  const candidates = useTrackPages({ sort: 'path', text: query, excludePlaylist: playlist?.id }, editor === 'add' && !!playlist);

  useEffect(() => {
    void changePlaylists({ type: 'list' }).catch((error) => setError(String(error))).finally(() => setLoaded(true));
  }, []);

  const selected = playlist && detail?.data
    ? { id: String(playlist.id), name: playlist.name, subtitle: null, tracks: detail.data }
    : undefined;

  const showEditor = (mode: Editor) => {
    setEditorError('');
    setName(mode === 'rename' ? playlist?.name ?? '' : '');
    setQuery('');
    setEditor(mode);
  };

  const run = async (operation: () => Promise<void>) => {
    const report = editor ? setEditorError : setError;
    setBusy(true);
    report('');
    try {
      await operation();
    } catch (error) {
      report(String(error));
    } finally {
      setBusy(false);
    }
  };

  const submit = () => {
    if (busy || !editor || editor === 'add') return;
    void run(async () => {
      if (editor === 'create') {
        const result = await changePlaylists({ type: 'create', name });
        const created = result[result.length - 1];
        if (created) open(String(created.id));
      } else if (editor === 'rename' && playlist) {
        await changePlaylists({ type: 'rename', id: playlist.id, name });
      } else if (editor === 'delete' && playlist) {
        await changePlaylists({ type: 'delete', id: playlist.id });
        open();
      }
      setEditor(undefined);
    });
  };

  return (
    <>
      {selected && playlist ? (
        <CollectionDetails
          key={selected.id}
          item={selected}
          kind='playlist'
          back={() => open()}
          actions={
            <>
              <Button size='small' disabled={busy} onClick={() => showEditor('add')}>Добавить треки</Button>
              <Button size='small' icon='Pen' disabled={busy} onClick={() => showEditor('rename')}>Переименовать</Button>
              <Button size='small' variant='danger' icon='Trash' disabled={busy} onClick={() => showEditor('delete')}>Удалить</Button>
              {busy && !editor && <LoadingIndicator label='Обновление плейлиста' />}
            </>
          }
          trackAction={(trackId) => {
            const index = playlist.trackIds.indexOf(trackId);
            return (
              <div className='flex gap-1'>
                <Button size='icon' icon='ChevronUp' aria-label='Переместить выше' disabled={busy || index <= 0}
                  onClick={() => void run(async () => { await changePlaylists({ type: 'move', id: playlist.id, trackId, later: false }); }) } />
                <Button size='icon' icon='ChevronDown' aria-label='Переместить ниже' disabled={busy || index >= playlist.trackIds.length - 1}
                  onClick={() => void run(async () => { await changePlaylists({ type: 'move', id: playlist.id, trackId, later: true }); }) } />
                <Button size='icon' icon='X' aria-label='Убрать из плейлиста' disabled={busy}
                  onClick={() => void run(async () => { await changePlaylists({ type: 'remove', id: playlist.id, trackId }); }) } />
              </div>
            );
          }}
        />
      ) : (
        <>
          <div className='playlist-actions'>
            <Button variant='primary' size='small' icon='Playlist' disabled={!loaded} onClick={() => showEditor('create')}>Создать плейлист</Button>
          </div>
          {loaded && summaries?.data && <CollectionList items={summaries.data} kind='playlist' empty='Пока нет плейлистов.' open={open} />}
          {(!loaded || (!summaries?.data && !summaries?.error) || (playlist && !detail?.data && !detail?.error))
            && <LoadingIndicator className='kit-message' label='Загрузка плейлистов' />}
        </>
      )}
      {(error || summaries?.error || detail?.error) && <p className='p-4 text-red-400' role='alert'>
        {error || summaries?.error || detail?.error}
      </p>}
      <Dialog open={editor !== undefined} busy={busy} onDismiss={() => setEditor(undefined)}
        title={{ create: 'Создать плейлист', rename: 'Переименовать плейлист', delete: 'Удалить плейлист', add: 'Добавить треки' }[editor ?? 'create']}
        description={editor === 'delete' ? `Удалить «${playlist?.name}»?` : undefined}>
        <form className='kit-dialog-form' onSubmit={(event) => { event.preventDefault(); submit(); }}>
          {(editor === 'create' || editor === 'rename') && (
            <Input icon='Playlist' aria-label='Название плейлиста' placeholder='Название плейлиста' value={name}
              maxLength={120} required disabled={busy} data-initial-focus
              aria-invalid={!!editorError} onInput={(event) => setName(event.currentTarget.value)} />
          )}
          {editor === 'add' && (
            <>
              <Input icon='Search' aria-label='Поиск трека' placeholder='Найти трек' value={query} data-initial-focus
                onInput={(event) => setQuery(event.currentTarget.value)} />
              {busy && <LoadingIndicator label='Добавление трека в плейлист' />}
              <div className='playlist-candidates'>
                {candidates.tracks.map((track) => (
                  <Track key={track.id} track={track} disabled={busy} onPlay={() => void run(async () => {
                    if (playlist) await changePlaylists({ type: 'add', id: playlist.id, trackId: track.id });
                  })} />
                ))}
                {candidates.loading && <LoadingIndicator label='Загрузка треков' />}
                {candidates.error && <p role='alert'>{candidates.error}</p>}
                {!candidates.tracks.length && !candidates.loading && !candidates.error && <p className='kit-message'>Нет подходящих треков.</p>}
                {candidates.hasMore && <Button size='small' pending={candidates.loadingMore} onClick={candidates.loadMore}>Показать ещё</Button>}
              </div>
            </>
          )}
          {editorError && <p className='kit-error' role='alert'>{editorError}</p>}
          <div className='kit-dialog-actions'>
            <Button variant={editor === 'add' ? 'primary' : 'secondary'} disabled={busy} onClick={() => setEditor(undefined)}>
              {editor === 'add' ? 'Готово' : 'Отмена'}
            </Button>
            {editor !== 'add' && (
              <Button type='submit' pending={busy} variant={editor === 'delete' ? 'danger' : 'primary'}
                disabled={busy || (editor !== 'delete' && !name.trim())}>
                {editor === 'delete' ? 'Удалить' : 'Сохранить'}
              </Button>
            )}
          </div>
        </form>
      </Dialog>
    </>
  );
}
