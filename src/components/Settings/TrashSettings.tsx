import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { Input } from '@/components/Input';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Artwork } from '@/components/MediaCard';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { useIntersection } from '@/hooks/useIntersection';
import { useTrashCover } from '@/hooks/useTrackCover';
import type { Track } from '@/types/Track';
import { formatTrackCount } from '@/utils/collections';

type Entry = { id: number; track: Track; removedAt: number; expiresAt: number };
type Trash = { retentionDays: number; entries: Entry[] };
type PurgeTarget = { type: 'entry'; id: number; name: string } | { type: 'all' };
type Operation = { type: 'load' | 'save' } | { type: 'restore'; id: number } | { type: 'purge'; target: PurgeTarget };

const date = new Intl.DateTimeFormat('ru', { dateStyle: 'medium' });
const dateTime = new Intl.DateTimeFormat('ru', { dateStyle: 'medium', timeStyle: 'short' });

function TrashRow({ entry, operation, restore, remove }: {
  entry: Entry;
  operation?: Operation;
  restore: () => void;
  remove: () => void;
}) {
  const ref = useRef<HTMLLIElement>(null);
  const visible = useIntersection(ref);
  const { coverSrc, clearCover } = useTrashCover(visible ? entry.id : undefined);
  const { track, removedAt, expiresAt } = entry;
  const artist = track.artists.join(', ') || 'Неизвестный исполнитель';
  return (
    <li ref={ref}>
      <Artwork className='track-trash-cover' icon='Track' src={coverSrc} onError={clearCover} />
      <div className='track-label'>
        <strong className='media-title' title={track.name}>{track.name}</strong>
        <span className='track-artist' title={artist}>{artist}</span>
        <small className='track-trash-date' title={`В корзине с ${dateTime.format(removedAt * 1000)}`}>
          Автоудаление {date.format(expiresAt * 1000)}
        </small>
      </div>
      <div className='track-trash-actions'>
        <Button size='icon' variant='ghost' icon='Refresh' title='Восстановить' aria-label={`Восстановить «${track.name}»`}
          pending={operation?.type === 'restore' && operation.id === entry.id} disabled={!!operation} onClick={restore} />
        <Button size='icon' variant='ghost' icon='Trash' className='track-trash-delete'
          title='Удалить навсегда' aria-label={`Удалить навсегда «${track.name}»`}
          pending={operation?.type === 'purge' && operation.target.type === 'entry' && operation.target.id === entry.id}
          disabled={!!operation} onClick={remove} />
      </div>
    </li>
  );
}

export function TrashSettings() {
  const [data, setData] = useState<Trash>();
  const [days, setDays] = useState('');
  const [open, setOpen] = useState(false);
  const [confirm, setConfirm] = useState<PurgeTarget>();
  const [operation, setOperation] = useState<Operation>();
  const [error, setError] = useState<string>();
  const next = Number(days);
  const valid = Number.isInteger(next) && next >= 1 && next <= 365;
  const entryCount = data?.entries.length ?? 0;
  const busy = operation?.type === 'restore' || operation?.type === 'purge';

  async function update(action: Operation) {
    setOperation(action);
    setError(undefined);
    try {
      try {
        if (action.type === 'save') await invoke('library_set_trash_retention', { days: next });
        if (action.type === 'restore') await invoke('library_restore_track', { id: action.id });
        if (action.type === 'purge') {
          const { target } = action;
          await invoke('library_purge_trash', { target: target.type === 'all' ? target : { type: 'entry', id: target.id } });
        }
      } finally {
        const result = await invoke<Trash>('library_trash_list');
        setData(result);
        if (action.type === 'load' || action.type === 'save') setDays(String(result.retentionDays));
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setOperation(undefined);
    }
  }
  useEffect(() => { void update({ type: 'load' }); }, []);

  return <SettingsSection title='Корзина треков'
    actions={<Button size='small' icon='Trash' disabled={!!operation} onClick={() => {
      setOpen(true);
      void update({ type: 'load' });
    }}>Открыть корзину</Button>}>
    <form className='trash-retention' onSubmit={(event) => {
      event.preventDefault();
      if (valid && !operation && data && next !== data.retentionDays) void update({ type: 'save' });
    }}>
      <label htmlFor='trash-retention-days'>Хранить треки, дней</label>
      <div className='trash-retention-controls'>
        <Input id='trash-retention-days' type='number' min={1} max={365} step={1}
          disabled={!data || !!operation} value={days}
          aria-describedby='trash-retention-note' onInput={(event) => setDays(event.currentTarget.value)} />
        <Button type='submit' size='small' pending={operation?.type === 'save'}
          disabled={!data || !!operation || !valid || next === data.retentionDays}>Сохранить</Button>
      </div>
      <p id='trash-retention-note'>Новый срок применяется ко всем трекам в корзине.</p>
    </form>
    {operation?.type === 'load' && !open && <LoadingIndicator label='Чтение корзины' />}
    {error && !open && <p className='kit-error' role='alert'>{error}</p>}
    {open && <Dialog open title='Корзина треков' className='track-trash-dialog' busy={busy} onDismiss={() => setOpen(false)}>
      <div className='track-trash-toolbar'>
        <span>{data ? formatTrackCount(entryCount) : ''}</span>
        <Button size='small' icon='Trash' disabled={!entryCount || !!operation}
          pending={operation?.type === 'purge' && operation.target.type === 'all'}
          onClick={() => setConfirm({ type: 'all' })}>Очистить корзину</Button>
      </div>
      {operation?.type === 'load' && <LoadingIndicator label='Загрузка корзины' />}
      {error && <p role='alert' className='kit-error'>{error}</p>}
      {data && !entryCount && operation?.type !== 'load' &&
        <div className='track-trash-empty'><Artwork icon='Trash' /><p>Корзина пуста</p></div>}
      {!!entryCount && <ul className='track-trash-list'>
        {data?.entries.map((entry) => <TrashRow key={entry.id} entry={entry} operation={operation}
          restore={() => void update({ type: 'restore', id: entry.id })}
          remove={() => setConfirm({ type: 'entry', id: entry.id, name: entry.track.name })} />)}
      </ul>}
    </Dialog>}
    {confirm && <Dialog open title={confirm.type === 'all' ? 'Очистить корзину?' : 'Удалить трек навсегда?'}
      description={confirm.type === 'all' ? 'Все файлы в корзине будут удалены без возможности восстановления.'
        : `Файл «${confirm.name}» будет удалён без возможности восстановления.`}
      onDismiss={() => setConfirm(undefined)}>
      <div className='kit-dialog-actions'>
        <Button data-initial-focus onClick={() => setConfirm(undefined)}>Отмена</Button>
        <Button variant='danger' onClick={() => {
          setConfirm(undefined);
          void update({ type: 'purge', target: confirm });
        }}>{confirm.type === 'all' ? 'Очистить корзину' : 'Удалить навсегда'}</Button>
      </div>
    </Dialog>}
  </SettingsSection>;
}
