import { ArtistProfilePage } from '@/components/ArtistProfilePage';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { useCollectionTracks } from '@/hooks/useLibrary';
import { createFileRoute } from '@tanstack/react-router';
import { CatalogCollectionPage } from '@/components/Collections';

export const Route = createFileRoute('/artists')({
  validateSearch: (search: Record<string, unknown>): { collection?: string; artist?: string } => ({
    collection: typeof search.collection === 'string' ? search.collection : undefined,
    artist: typeof search.artist === 'string' ? search.artist.trim().slice(0, 512) : undefined,
  }),
  component: CollectionPage,
});

function CollectionPage() {
  const { collection, artist } = Route.useSearch();
  const navigate = Route.useNavigate();
  if (artist) return <ExternalArtist key={artist} name={artist} />;
  return <CatalogCollectionPage kind='artist' id={collection}
    open={(id) => void navigate({ search: { collection: id } })} />;
}


function ExternalArtist({ name }: { name: string }) {
  const id = name.toLowerCase();
  const tracks = useCollectionTracks('artist', id);
  if (tracks?.error) return <p className='kit-message' role='alert'>{tracks.error}</p>;
  if (!tracks?.data) return <LoadingIndicator className='kit-message' label='Загрузка исполнителя' />;
  return <ArtistProfilePage artist={{ id, name, subtitle: null, tracks: tracks.data }} />;
}
