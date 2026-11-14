import { Button } from '@/components/Button';
import { Select, type SelectOption } from '@/components/Select';
import { type LibrarySort, useTrackViewStore } from '@/stores/trackViewStore';

const sorts: SelectOption<LibrarySort>[] = [
  { value: 'name', label: 'По названию' },
  { value: 'releaseYear', label: 'По дате выхода' },
  { value: 'added', label: 'По дате добавления' },
  { value: 'created', label: 'По дате создания' },
  { value: 'artist', label: 'По исполнителю' },
  { value: 'duration', label: 'По длительности' },
];

export function TracksToolbar({ page }: { page: string }) {
  const { layout, sort, setLayout, setSort } = useTrackViewStore();
  const options = sorts.filter(({ value }) => value === 'name'
    || (value === 'releaseYear' && page === '/')
    || (value === 'added' && (page === '/' || page === '/tracks'))
    || (value === 'created' && page === '/playlists')
    || (value === 'artist' && (page === '/tracks' || page === '/'))
    || (value === 'duration' && page !== '/artists'));

  return (
    <div className='tracks-toolbar'>
      <Select label='Сортировка' value={options.some(({ value }) => value === sort) ? sort : 'name'}
        options={options} onChange={setSort} className='track-sort' />
      <div className='view-toggle' role='group' aria-label='Вид медиатеки'>
        <Button variant='ghost' size='icon' icon='List' aria-label='Список' aria-pressed={layout === 'list'} onClick={() => setLayout('list')} />
        <Button variant='ghost' size='icon' icon='Grid' aria-label='Сетка' aria-pressed={layout === 'grid'} onClick={() => setLayout('grid')} />
      </div>
    </div>
  );
}
