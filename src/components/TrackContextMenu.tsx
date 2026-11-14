import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Dialog } from '@/components/Dialog';
import { addTrackToQueue } from '@/stores/playerStore';
import { tracksByPaths } from '@/hooks/useLibrary';
import type { Track } from '@/types/Track';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';
import { checkPlayerVideo, showPlayerVideo } from '@/utils/playerVideo';
import { toast } from '@/utils/toast';

type FileInfo = { size: number; format: string };
const fileSize = new Intl.NumberFormat('ru', { style: 'unit', unit: 'megabyte', maximumFractionDigits: 2 });

export function TrackContextMenu() {
  const menu = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLElement>();
  const path = useRef('');
  const [details, setDetails] = useState<{
    data?: { track: Track; file: FileInfo };
    error?: string;
  }>();
  const detailRequest = useRef(0);

  useEffect(() => {
    function open(event: MouseEvent) {
      event.preventDefault();
      const target = event.target instanceof Element ? event.target : null;
      const track = target?.closest<HTMLElement>('[data-track-path]');
      const panel = menu.current;
      if (!panel) return;
      if (panel.matches(':popover-open')) panel.hidePopover();
      if (!track?.dataset.trackPath) return;
      path.current = track.dataset.trackPath;
      panel.toggleAttribute('data-has-video', track.hasAttribute('data-local-video'));
      const video = track.dataset.playerVideo;
      const trackPath = path.current;
      if (video && !track.hasAttribute('data-local-video')) void checkPlayerVideo(video).then((available) => {
        if (path.current === trackPath && panel.matches(':popover-open')) {
          panel.toggleAttribute('data-has-video', available);
        }
      });
      opener.current = target?.closest<HTMLElement>('button, a') ?? track;
      const rect = track.getBoundingClientRect();
      const keyboard = event.clientX === 0 && event.clientY === 0;
      panel.showPopover();
      panel.style.left = `${Math.max(8, Math.min(keyboard ? rect.left : event.clientX, innerWidth - panel.offsetWidth - 8))}px`;
      panel.style.top = `${Math.max(8, Math.min(keyboard ? rect.bottom : event.clientY, innerHeight - panel.offsetHeight - 8))}px`;
      panel.querySelector('button')?.focus();
    }
    function dismissOutside(event: PointerEvent) {
      const panel = menu.current;
      // The release that follows contextmenu can be part of the opening gesture.
      if (event.button === 0 && !event.ctrlKey && panel?.matches(':popover-open')
        && !event.composedPath().includes(panel)) {
        panel.hidePopover();
      }
    }
    document.addEventListener('contextmenu', open);
    document.addEventListener('pointerdown', dismissOutside, true);
    return () => {
      document.removeEventListener('contextmenu', open);
      document.removeEventListener('pointerdown', dismissOutside, true);
    };
  }, []);

  const run = (action: (path: string) => Promise<void>, error: string) => {
    menu.current?.hidePopover();
    opener.current?.focus();
    let notice: ReturnType<typeof toast.loading> | undefined;
    const progress = window.setTimeout(() => {
      notice = toast.loading('Действие с треком');
    }, 250);
    void action(path.current).catch((reason) => toast.error(error, reason)).finally(() => {
      window.clearTimeout(progress);
      if (notice !== undefined) toast.dismiss(notice);
    });
  };

  return <>
    <div ref={menu} className='track-context-menu' popover='manual' aria-label='Действия с треком'
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.stopPropagation();
          menu.current?.hidePopover();
          opener.current?.focus();
        }
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault();
          const buttons = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(.track-context-video), [data-has-video] .track-context-video') ?? []);
          const index = buttons.findIndex((button) => button === document.activeElement);
          const step = event.key === 'ArrowDown' ? 1 : -1;
          buttons[(index + step + buttons.length) % buttons.length]?.focus();
        }
      }}>
      <Button variant='ghost' size='small' icon='Queue' onClick={() => run(async (path) => {
        const [track] = await tracksByPaths([path]);
        const snapshot = await addTrackToQueue(track ?? { path, replayGainDb: null });
        if (!snapshot.error) toast.message('Трек добавлен в очередь');
      }, 'Не удалось добавить трек в очередь')}>Добавить в очередь</Button>
      <Button variant='ghost' size='small' icon='Open' onClick={() => run(
        (path) => invoke('reveal_track', { path }), 'Не удалось показать файл',
      )}>Показать в папке</Button>
      <Button className='track-context-video' variant='ghost' size='small' icon='Play' onClick={() => {
        menu.current?.hidePopover();
        opener.current?.focus();
        const trackPath = path.current;
        void tracksByPaths([trackPath]).then(([track]) => track && showPlayerVideo(track))
          .catch((error) => toast.error('Не удалось открыть видео', error));
      }}>Смотреть видео</Button>
      <Button variant='ghost' size='small' icon='Track' onClick={() => {
        menu.current?.hidePopover();
        opener.current?.focus();
        const trackPath = path.current;
        const request = ++detailRequest.current;
        setDetails({});
        void Promise.all([
          tracksByPaths([trackPath]),
          invoke<FileInfo>('track_file_info', { path: trackPath }),
        ]).then(([[track], file]) => {
          if (request !== detailRequest.current) return;
          setDetails(track
            ? { data: { track, file } }
            : { error: 'Трек отсутствует в медиатеке.' });
        }).catch((reason) => {
          if (request === detailRequest.current) setDetails({ error: String(reason) });
        });
      }}>Сведения о файле</Button>
    </div>
    <Dialog open={!!details} title='Сведения о файле' onDismiss={() => {
      detailRequest.current += 1;
      setDetails(undefined);
    }}>
      {details && !details.data && !details.error && <LoadingIndicator label='Загрузка сведений о файле' />}
      {details?.error && <p role='alert' className='kit-error'>{details.error}</p>}
      {details?.data && <dl className='track-file-details'>
        <dt>Название</dt><dd>{details.data.track.name}</dd>
        <dt>Исполнители</dt><dd>{details.data.track.artists.join(', ') || '—'}</dd>
        <dt>Альбом</dt><dd>{details.data.track.album || '—'}</dd>
        <dt>Год</dt><dd>{details.data.track.year ?? '—'}</dd>
        <dt>Жанр</dt><dd>{details.data.track.genre || '—'}</dd>
        <dt>Длительность</dt><dd>{details.data.track.duration ? formatPlaybackTime(details.data.track.duration) : '—'}</dd>
        <dt>Формат</dt><dd>{details.data.file.format || '—'}</dd>
        <dt>Размер</dt><dd>{fileSize.format(details.data.file.size / 1_000_000)}</dd>
        <dt>Файл</dt><dd>{details.data.track.path}</dd>
      </dl>}
    </Dialog>
  </>;
}
