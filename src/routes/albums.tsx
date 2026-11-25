import { createFileRoute } from '@tanstack/react-router';
import { CatalogCollectionPage, ExternalAlbumPage } from '@/components/Collections';
import type { AlbumRelease, ReleaseReference } from '@/hooks/useRemoteCatalog';

function isReleaseReference(value: unknown): value is ReleaseReference {
  if (!value || typeof value !== 'object' || !('provider' in value) || !('id' in value) || typeof value.id !== 'string') return false;
  if (value.provider === 'musicBrainz') return /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu.test(value.id);
  return (value.provider === 'itunes' || value.provider === 'deezer' || value.provider === 'jioSaavn') && /^\d{1,19}$/u.test(value.id);
}

function coverUrl(value: unknown) {
  if (typeof value !== 'string' || value.length > 2048) return null;
  let url: URL;
  try { url = new URL(value); } catch { return null; }
  const trusted = url.hostname.endsWith('.mzstatic.com') || url.hostname.endsWith('.dzcdn.net')
    || ['coverartarchive.org', 'images.genius.com', 'upload.wikimedia.org', 'commons.wikimedia.org',
      'lh3.googleusercontent.com', 'yt3.googleusercontent.com', 'i.ytimg.com', 'c.saavncdn.com'].includes(url.hostname);
  return trusted && url.protocol === 'https:' && !url.username && !url.password && !url.port ? url.href : null;
}

function albumRelease(value: unknown): AlbumRelease | undefined {
  if (!value || typeof value !== 'object') return;
  const { reference, alternates, title, kind, date, artwork, artist, artistId } = value as Partial<AlbumRelease>;
  if (!isReleaseReference(reference) || typeof title !== 'string' || !title.trim() || title.length > 512
    || (kind !== 'album' && kind !== 'single' && kind !== 'ep' && kind !== 'other')) return;
  return {
    reference: { provider: reference.provider, id: reference.id },
    alternates: Array.isArray(alternates) ? alternates.slice(0, 5).filter(isReleaseReference).map(({ provider, id }) => ({ provider, id })) : undefined,
    title,
    kind,
    date: typeof date === 'string' && /^\d{4}(?:-\d{2}){0,2}$/u.test(date) ? date : null,
    artwork: coverUrl(artwork),
    artist: typeof artist === 'string' ? artist.slice(0, 512) : '',
    artistId: typeof artistId === 'string' ? artistId.slice(0, 512) : undefined,
  };
}

export function validateAlbumSearch(search: Record<string, unknown>): { collection?: string; release?: AlbumRelease } {
  return {
    collection: typeof search.collection === 'string' ? search.collection : undefined,
    release: albumRelease(search.release),
  };
}

export const Route = createFileRoute('/albums')({
  validateSearch: validateAlbumSearch,
  component: CollectionPage,
});

function CollectionPage() {
  const { collection, release } = Route.useSearch();
  const navigate = Route.useNavigate();
  if (release) return <ExternalAlbumPage key={`${release.reference.provider}:${release.reference.id}`} release={release} collection={collection} />;
  return <CatalogCollectionPage kind='album' id={collection}
    open={(id) => void navigate({ search: { collection: id } })} />;
}
