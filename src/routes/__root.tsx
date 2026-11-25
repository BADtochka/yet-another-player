import { getCurrentWindow } from '@tauri-apps/api/window';
import { createRootRoute, Outlet, useLocation, useNavigate } from '@tanstack/react-router';

import { useEffect, useRef, useState } from 'preact/hooks';
import { usePageSwipe } from '@/hooks/usePageSwipe';
import { listenForCatalogChanges } from '@/hooks/useLibrary';
import { AppToaster } from '@/components/AppToaster';
import { FlexLogo } from '@/components/FlexLogo';
import { Icon } from '@/components/Icons/Icon';
import { NavBar } from '@/components/NavBar';
import { PlayerVideo } from '@/components/PlayerVideo';
import { PlayerControls } from '@/components/PlayerControls';
import { SyncAccessPrompt } from '@/components/SyncAccessPrompt';
import { TrackContextMenu } from '@/components/TrackContextMenu';
import { VolumeGesture } from '@/components/VolumeGesture';
import { YtDlpGatekeeperDialog } from '@/components/YtDlpFeedback';
import { startLibrarySync } from '@/stores/librarySyncStore';
import { savePlayerStateOnExit } from '@/stores/playerStore';
import { installGlobalLogHandlers } from '@/utils/appLogs';
import { checkForAppUpdates } from '@/utils/checkForUpdates';
import { prepareStore } from '@/utils/prepareStore';
import { usePlayerStore, useSettingsStore } from '@/stores';
import { getExternalToolStatus } from '@/utils/externalTools';
import { canSearchLyrics, getLocalLyrics, localLyricsText, lookupPreferredLyrics, saveProviderLyrics } from '@/utils/lyrica';
import { usePlayerVideo } from '@/utils/playerVideo';
import { toast } from '@/utils/toast';

let closingAfterSave = false;

const RootLayout = () => {
  const [storeLoaded, setStoreLoaded] = useState(false);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const videoPath = usePlayerVideo((state) => state.request?.path);
  const currentPath = usePlayerStore((state) => state.currentTrackPath);
  const currentTrack = usePlayerStore((state) => state.currentTrack);
  const playing = usePlayerStore((state) => state.playing);
  const lyricsEnabled = useSettingsStore(canSearchLyrics);
  const lyricsSource = useSettingsStore((state) => state.lyricsSource);
  const separateVideoLyrics = useSettingsStore((state) => state.separateVideoLyrics);
  const suspendHiddenVideo = useSettingsStore((state) => state.suspendHiddenVideo);
  const [pageHidden, setPageHidden] = useState(() => document.hidden);
  const videoSuspended = suspendHiddenVideo && pageHidden;
  const lastLyricsLookup = useRef<string>();
  const lyricsWithVideo = pathname === '/lyrics' && !videoSuspended && !!videoPath && videoPath === currentPath;
  const isSetup = pathname === '/setup';
  const hasToolbar = !isSetup && pathname !== '/settings';
  const shell = useRef<HTMLDivElement>(null);
  const page = useRef<HTMLDivElement>(null);
  const nativePageSwipe = '__YAP_NATIVE_PAGE_SWIPE__' in window;
  const swipeFeedback = usePageSwipe(shell, page, storeLoaded && !isSetup, nativePageSwipe);

  useEffect(() => {
    const updateVisibility = () => setPageHidden(document.hidden);
    document.addEventListener('visibilitychange', updateVisibility);
    updateVisibility();
    return () => document.removeEventListener('visibilitychange', updateVisibility);
  }, []);

  useEffect(() => {
    if (!lyricsEnabled || !playing || !currentTrack || currentTrack.path !== currentPath) return;
    const key = JSON.stringify([currentTrack.path, currentTrack.name, currentTrack.artists, lyricsSource]);
    if (lastLyricsLookup.current === key) return;
    const timer = window.setTimeout(() => {
      lastLyricsLookup.current = key;
      void (async () => {
        const local = await getLocalLyrics(currentTrack.path);
        if (!local || localLyricsText(currentTrack, local) || !canSearchLyrics(useSettingsStore.getState())) return;
        const result = await lookupPreferredLyrics(currentTrack);
        if (result && canSearchLyrics(useSettingsStore.getState()) && useSettingsStore.getState().lyricsSource === lyricsSource) {
          await saveProviderLyrics(currentTrack.path, result.content, local.fetched);
        }
      })().catch((error) => console.warn('Could not find track lyrics', error));
    }, 400);
    return () => window.clearTimeout(timer);
  }, [currentPath, currentTrack, playing, lyricsEnabled, lyricsSource]);

  useEffect(() => {
    if (storeLoaded && !isSetup) return startLibrarySync();
  }, [storeLoaded, isSetup]);

  const warmApp = async () => {
    const { hasMusicFolders } = await prepareStore();
    void getExternalToolStatus('lyrica').catch((error) => console.warn('Could not check local Lyrica', error));
    setStoreLoaded(true);

    if (!hasMusicFolders && pathname !== '/setup') {
      await navigate({ to: '/setup' });
    }
  };

  useEffect(() => {
    installGlobalLogHandlers();
    void checkForAppUpdates();

    const catalogListener = listenForCatalogChanges().catch(() => undefined);
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

    void catalogListener.then(warmApp).catch((error) => {
      console.error('Failed to prepare app store', error);
      toast.error('Не удалось загрузить библиотеку', error);
      setStoreLoaded(true);

      if (pathname !== '/setup') {
        void navigate({ to: '/setup' });
      }
    });

    return () => {
      unlistenClose?.();
      void catalogListener.then((unlisten) => unlisten?.());
    };
  }, []);

  return (
    <>
      <div ref={shell} className='app-shell' data-setup={isSetup} data-toolbar={hasToolbar}
        data-video-lyrics={lyricsWithVideo && separateVideoLyrics ? 'separate' : undefined}>
        {!isSetup && <NavBar />}
        <main className='app-content' data-lyrics-with-video={lyricsWithVideo}>
          <div ref={page} className='app-page'>{storeLoaded ? <Outlet /> : <FlexLogo />}</div>
        </main>
        <PlayerVideo suspended={videoSuspended} />
        <PlayerControls ready={storeLoaded} />
        {storeLoaded && <VolumeGesture />}
        <div ref={swipeFeedback} className='page-swipe-feedback' aria-hidden='true'>
          <span className='page-swipe-arrow' data-edge='left'><Icon name='ChevronUp' /></span>
          <span className='page-swipe-arrow' data-edge='right'><Icon name='ChevronUp' /></span>
        </div>
      </div>
      <TrackContextMenu />
      <AppToaster />
      <YtDlpGatekeeperDialog />
      {storeLoaded && <SyncAccessPrompt />}
    </>
  );
};

export const Route = createRootRoute({ component: RootLayout });
