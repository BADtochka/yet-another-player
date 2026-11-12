import { useLocation, useNavigate } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'preact/hooks';
import { TracksToolbar } from '@/components/TracksToolbar';
import { Artwork } from '@/components/MediaCard';
import { Input } from '@/components/Input';
import { Tab } from '@/components/Tab';
import { addSearchHistory, useSearchHistory } from '@/stores/searchHistory';

export function NavBar() {
  const location = useLocation();
  const navigate = useNavigate();
  const [query, setQuery] = useState('');
  const [showHistory, setShowHistory] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const history = useSearchHistory((state) => state.entries);
  const { pathname } = location;
  const params = new URLSearchParams(location.searchStr);
  const currentQuery = pathname === '/search' ? params.get('q') ?? '' : '';

  useEffect(() => {
    setQuery(currentQuery);
    setShowHistory(false);
  }, [pathname, currentQuery]);

  const search = (value: string) => {
    const q = value.trim();
    const repeating = pathname === '/search' && q !== '' && q === currentQuery;
    const previousRequest = Number(params.get('request'));
    const request = repeating && Number.isSafeInteger(previousRequest) && previousRequest >= 0 && previousRequest < Number.MAX_SAFE_INTEGER
      ? previousRequest + 1 : undefined;
    if (q) addSearchHistory({ query: q });
    setQuery(q);
    setShowHistory(false);
    void navigate({ to: '/search', search: { q, tab: undefined, request }, replace: repeating && !params.has('tab') });
  };
  const clear = () => {
    setQuery('');
    if (currentQuery) search('');
    input.current?.focus();
    setShowHistory(true);
  };

  return (
    <>
      <nav className='app-sidebar' aria-label='Медиатека'>
        <Tab to='/' icon='Album' label='Альбомы' />
        <Tab to='/tracks' icon='Track' label='Треки' />
        <Tab to='/artists' icon='User' label='Исполнители' />
        <Tab to='/playlists' icon='Playlist' label='Плейлисты' />
        <Tab to='/settings' icon='Settings' label='Настройки' className='mt-auto' />
      </nav>
      {pathname !== '/settings' && (
        <div className='app-toolbar'>
          <form className='app-search' role='search' onSubmit={(event) => { event.preventDefault(); search(query); }}
            onFocusOut={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setShowHistory(false);
            }}
            onKeyDown={(event) => { if (event.key === 'Escape') setShowHistory(false); }}>
            <Input ref={input} placeholder='Что-то ищете?' aria-label='Поиск' value={query} submit onClear={clear}
              onFocus={() => setShowHistory(true)}
              onClick={() => setShowHistory(true)}
              onInput={(event) => { setQuery(event.currentTarget.value); setShowHistory(false); }} />
            {showHistory && history.length > 0 && (
              <div className='search-history'>
                <p>История поиска</p>
                {history.map((item) => (
                  <button key={item.trackId === undefined ? `query:${item.query}` : `track:${item.trackId}`} type='button'
                    onMouseDown={(event) => event.preventDefault()} onClick={() => search(item.query)}>
                    <Artwork icon={item.trackId === undefined ? 'Search' : 'Track'} />
                    <span className='track-label'>
                      <span className='media-title'>{item.title ?? item.query}</span>
                      {item.artist && <span className='track-artist'>{item.artist}</span>}
                    </span>
                  </button>
                ))}
              </div>
            )}
          </form>
          {['/', '/tracks', '/artists', '/playlists'].includes(pathname) && !params.has('collection') && <TracksToolbar page={pathname} />}
        </div>
      )}
    </>
  );
}
