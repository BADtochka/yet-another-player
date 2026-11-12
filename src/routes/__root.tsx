import { getCurrentWindow } from '@tauri-apps/api/window';
import { createRootRoute, Outlet, useLocation, useNavigate } from '@tanstack/react-router';

import { useEffect, useState } from 'preact/hooks';
import { AppToaster } from '@/components/AppToaster';
import { FlexLogo } from '@/components/FlexLogo';
import { NavBar } from '@/components/NavBar';
import { PlayerControls } from '@/components/PlayerControls';
import { WindowBar } from '@/components/WindowBar';
import { YtDlpGatekeeperDialog } from '@/components/YtDlpFeedback';
import { savePlayerStateOnExit } from '@/stores/playerStore';
import { installGlobalLogHandlers } from '@/utils/appLogs';
import { checkForAppUpdates } from '@/utils/checkForUpdates';
import { prepareStore } from '@/utils/prepareStore';
import { toast } from '@/utils/toast';

let closingAfterSave = false;

const RootLayout = () => {
  const [storeLoaded, setStoreLoaded] = useState(false);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const isSetup = pathname === '/setup';
  const hasToolbar = !isSetup && pathname !== '/settings';

  const warmApp = async () => {
    const { hasMusicFolders } = await prepareStore();
    setStoreLoaded(true);

    if (!hasMusicFolders && pathname !== '/setup') {
      await navigate({ to: '/setup' });
    }
  };

  useEffect(() => {
    installGlobalLogHandlers();
    void checkForAppUpdates();

    let unlistenClose: (() => void) | undefined;

    void getCurrentWindow()
      .onCloseRequested(async (event) => {
        if (closingAfterSave) {
          return;
        }

        event.preventDefault();
        closingAfterSave = true;

        try {
          await savePlayerStateOnExit();
        } catch (error) {
          console.error('Failed to save player state on exit', error);
        }
        await getCurrentWindow().close().catch((error) => {
          closingAfterSave = false;
          console.error('Failed to close window', error);
        });
      })
      .then((unlisten) => {
        unlistenClose = unlisten;
      });

    void warmApp().catch((error) => {
      console.error('Failed to prepare app store', error);
      toast.error('Не удалось загрузить библиотеку', error);
      setStoreLoaded(true);

      if (pathname !== '/setup') {
        void navigate({ to: '/setup' });
      }
    });

    return () => {
      unlistenClose?.();
    };
  }, []);

  return (
    <>
      <div className='app-shell' data-setup={isSetup} data-toolbar={hasToolbar}>
        <WindowBar />
        {!isSetup && <NavBar />}
        <main className='app-content'>
          {storeLoaded ? <Outlet /> : <FlexLogo />}
        </main>
        <PlayerControls />
      </div>
      <AppToaster />
      <YtDlpGatekeeperDialog />
    </>
  );
};

export const Route = createRootRoute({ component: RootLayout });
