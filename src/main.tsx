import { RouterProvider, createRouter } from '@tanstack/react-router';
import { render } from 'preact';
import '@/styles.css';

import { routeTree } from '@/routeTree.gen';

const router = createRouter({ routeTree });

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

render(<RouterProvider router={router} />, document.getElementById('root')!);
