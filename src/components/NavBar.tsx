import { Album } from '@/components/Icons/Album';
import { Playlist } from '@/components/Icons/Playlist';
import { Settings } from '@/components/Icons/Settings';
import { Track } from '@/components/Icons/Track';
import { User } from '@/components/Icons/User';
import { Tab } from '@/components/Tab';

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
      <Tab to='/settings' icon={<Settings size={20} />} className='ml-auto'>
        Настройки
      </Tab>
    </nav>
  );
};
