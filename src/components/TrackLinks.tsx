import { Link } from '@tanstack/react-router';
import type { ComponentChildren } from 'preact';
import type { Track } from '@/types/Track';

export function TrackAlbumLink({ track, children }: { track: Track; children: ComponentChildren }) {
  const album = track.album?.trim() || track.name;
  return <Link className='track-navigation-link' to='/albums' search={{ collection: track.albumId }}
    aria-label={`Открыть альбом «${album}»`}>{children}</Link>;
}

export function TrackArtistLinks({ artists, allowExternal = false }: { artists: string[]; allowExternal?: boolean }) {
  if (!artists.length) return <>Неизвестный исполнитель</>;
  return <>{artists.map((artist, index) => (
    <span key={index}>
      {index > 0 && ', '}
      <Link className='track-navigation-link' to='/artists'
        search={allowExternal ? { artist } : { collection: artist.trim().toLowerCase() }}>
        {artist}
      </Link>
    </span>
  ))}</>;
}
