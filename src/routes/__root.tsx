import { createRootRoute, Outlet, useLocation, useNavigate } from '@tanstack/react-router';
import { TanStackRouterDevtools } from '@tanstack/react-router-devtools';

import { useEffect, useState } from 'preact/hooks';
import { FlexLogo } from '@/components/FlexLogo';
import { NavBar } from '@/components/NavBar';
import { PlayerControls } from '@/components/PlayerControls';
import { WindowBar } from '@/components/WindowBar';
import { checkForAppUpdates } from '@/utils/checkForUpdates';
import { prepareStore } from '@/utils/prepareStore';

const RootLayout = () => {
  const [storeLoaded, setStoreLoaded] = useState(false);
  const navigate = useNavigate();
  const { pathname } = useLocation();

  const warmApp = async () => {
    const { musicPath } = await prepareStore();
    setStoreLoaded(true);

    if (!musicPath && pathname !== '/setup') {
      await navigate({ to: '/setup' });
    }
  };

  useEffect(() => {
    void checkForAppUpdates();

    void warmApp().catch((error) => {
      console.error('Failed to prepare app store', error);
      setStoreLoaded(true);

      if (pathname !== '/setup') {
        void navigate({ to: '/setup' });
      }
    });
  }, []);

  return (
    <>
      <div className='flex h-screen flex-col overflow-hidden gap-4'>
        <WindowBar />
        <NavBar />
        <div className='flex min-h-0 flex-1 overflow-hidden pl-4 [&>div]:overflow-auto [&>div]:pr-6'>
          {storeLoaded ? <Outlet /> : <FlexLogo />}
        </div>
        <PlayerControls />
      </div>
      <TanStackRouterDevtools />
    </>
  );
};

export const Route = createRootRoute({ component: RootLayout });
