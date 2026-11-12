import { createFileRoute } from '@tanstack/react-router';
import { CatalogCollectionPage } from '@/components/Collections';

export const Route = createFileRoute('/artists')({
  validateSearch: (search: Record<string, unknown>) => ({
    collection: typeof search.collection === 'string' ? search.collection : undefined,
  }),
  component: CollectionPage,
});

function CollectionPage() {
  const { collection } = Route.useSearch();
  const navigate = Route.useNavigate();
  return <CatalogCollectionPage kind='artist' id={collection}
    open={(id) => void navigate({ search: { collection: id } })} />;
}
