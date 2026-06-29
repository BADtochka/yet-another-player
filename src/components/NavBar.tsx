import { Album } from './Icons/Album';
import { Playlist } from './Icons/Playlist';
import { Track } from './Icons/Track';
import { User } from './Icons/User';
import { Tab } from './Tab';

export const NavBar = () => {
  return (
    <nav className='flex flex-wrap gap-3 px-4 font-onest text-white'>
      <Tab to='/' icon={<Album size={20} />}>
        Альбомы
      </Tab>
      <Tab to='/tracks' icon={<Track size={20} />}>
        Треки
      </Tab>
      <Tab to='/artists' icon={<User size={20} />}>
        Исполнители
      </Tab>
      <Tab to='/playlists' icon={<Playlist size={20} />}>
        Плейлисты
      </Tab>
    </nav>
  );
};
