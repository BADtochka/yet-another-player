import { createFileRoute, redirect } from '@tanstack/react-router';
import { validateAlbumSearch } from './albums';

export const Route = createFileRoute('/')({
  validateSearch: validateAlbumSearch,
  beforeLoad: ({ search }) => {
    if (search.collection || search.release) {
      throw redirect({ to: '/albums', search, replace: true });
    }
    throw redirect({ to: '/tracks', replace: true });
  },
});
