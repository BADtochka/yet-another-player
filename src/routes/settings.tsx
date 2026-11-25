import { Button } from '@/components/Button';
import { BetaBadge } from '@/components/Settings/BetaBadge';
import { CatalogSettings, LyricsSourceSettings } from '@/components/Settings/ContentSourcesSettings';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { DiagnosticsSettings } from '@/components/Settings/DiagnosticsSettings';
import { ExternalToolsSettings } from '@/components/Settings/ExternalToolsSettings';
import { MediaFoldersSettings } from '@/components/Settings/MediaFoldersSettings';
import { SelectSettings } from '@/components/Settings/SelectSettings';
import { SwitchSettings } from '@/components/Settings/SwitchSettings';
import { TransitionSettings } from '@/components/Settings/TransitionSettings';
import { TrashSettings } from '@/components/Settings/TrashSettings';
import { UpdateSettings } from '@/components/Settings/UpdateSettings';
import { VolumeGestureSettings } from '@/components/Settings/VolumeGestureSettings';
import { useSettingsStore } from '@/stores';
import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';

const categories = [
  { id: 'audio', label: 'Аудио', component: AudioSettings },
  { id: 'video', label: 'Видео и тексты', component: VideoSettings },
  { id: 'library', label: 'Медиатека', component: LibrarySettings },
  { id: 'catalog', label: 'Каталоги', component: CatalogSettings },
  { id: 'tools', label: 'Инструменты', component: ExternalToolsSettings },
  { id: 'app', label: 'Приложение', component: AppSettings },
] as const;

type SettingsCategory = (typeof categories)[number]['id'];

export const Route = createFileRoute('/settings')({
  validateSearch: (search: Record<string, unknown>): { category?: SettingsCategory } => ({
    category: categories.find(({ id }) => id === search.category)?.id,
  }),
  component: RouteComponent,
});

function RouteComponent() {
  const { category } = Route.useSearch();
  const nav = useRef<HTMLElement>(null);
  const groups = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<SettingsCategory>(category ?? 'audio');

  const scrollToCategory = (id: SettingsCategory, animate: boolean) => {
    const bar = nav.current;
    const content = groups.current;
    const viewport = bar?.closest('main');
    const section = content?.querySelector(`#settings-${id}`);
    if (!bar || !content || !viewport || !section) return;
    const gap = parseFloat(getComputedStyle(content).rowGap);
    const top = viewport.scrollTop + section.getBoundingClientRect().top - viewport.getBoundingClientRect().top;
    const behavior = animate && !matchMedia('(prefers-reduced-motion: reduce)').matches ? 'smooth' : 'instant';
    viewport.scrollTo({ top: id === 'audio' ? 0 : top - bar.offsetHeight - gap, behavior });
  };

  useLayoutEffect(() => {
    scrollToCategory(category ?? 'audio', false);
  }, [category]);

  useEffect(() => {
    const bar = nav.current;
    const content = groups.current;
    const viewport = bar?.closest('main');
    if (!bar || !content || !viewport) return;
    const sections = [...content.children];
    const update = () => {
      const boundary = bar.getBoundingClientRect().bottom + parseFloat(getComputedStyle(content).rowGap) + 1;
      const atEnd = viewport.scrollTop > 0 && viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - 1;
      const next = sections.findIndex((item) => item.getBoundingClientRect().top > boundary);
      const section = atEnd || next === -1 ? sections.length - 1 : Math.max(0, next - 1);
      setActive(categories[section].id);
    };
    const resize = new ResizeObserver(update);
    resize.observe(bar);
    resize.observe(content);
    resize.observe(viewport);
    viewport.addEventListener('scroll', update, { passive: true });
    update();
    return () => {
      viewport.removeEventListener('scroll', update);
      resize.disconnect();
    };
  }, []);

  return <div className='settings-page'>
    <nav ref={nav} className='settings-categories' aria-label='Категории настроек'>
      {categories.map(({ id, label }) => <Button key={id} size='small' variant={active === id ? 'primary' : 'secondary'}
        aria-controls={`settings-${id}`} aria-current={active === id ? 'location' : undefined}
        onClick={() => scrollToCategory(id, true)}>
        {label}{id === 'video' && <BetaBadge />}
      </Button>)}
    </nav>
    <div ref={groups} className='settings-groups'>
      {categories.map(({ id, component: Category }) => <div key={id} id={`settings-${id}`} className='settings-group'>
        <Category />
      </div>)}
    </div>
  </div>;
}

function AudioSettings() {
  const volumeNormEnabled = useSettingsStore((state) => state.volumeNormEnabled);
  const setVolumeNormEnabled = useSettingsStore((state) => state.setVolumeNormEnabled);
  const volumeNormLevel = useSettingsStore((state) => state.volumeNormLevel);
  const setVolumeNormLevel = useSettingsStore((state) => state.setVolumeNormLevel);
  const showAudioInfo = useSettingsStore((state) => state.showAudioInfo);
  const setShowAudioInfo = useSettingsStore((state) => state.setShowAudioInfo);
  const restartOnPrevious = useSettingsStore((state) => state.restartOnPrevious);
  const setRestartOnPrevious = useSettingsStore((state) => state.setRestartOnPrevious);

  return <SettingsSection title='Воспроизведение аудио'>
    <SwitchSettings label='Показывать формат и качество аудио в плеере'
      checked={showAudioInfo} onChange={setShowAudioInfo} />
    <SwitchSettings label='По команде «Предыдущий трек» возвращаться к началу после 5 секунд'
      checked={restartOnPrevious} onChange={setRestartOnPrevious} />
    <SwitchSettings label='Установить одну громкость для всех треков'
      checked={volumeNormEnabled} onChange={setVolumeNormEnabled}>
      <SelectSettings label='Уровень громкости' options={[
        { label: 'Тихо', value: 'quiet' },
        { label: 'Нормально', value: 'normal' },
        { label: 'Громко', value: 'loud' },
      ]} value={volumeNormLevel} onChange={setVolumeNormLevel} />
    </SwitchSettings>
    <VolumeGestureSettings />
    <TransitionSettings />
  </SettingsSection>;
}

function VideoSettings() {
  const separateVideoLyrics = useSettingsStore((state) => state.separateVideoLyrics);
  const setSeparateVideoLyrics = useSettingsStore((state) => state.setSeparateVideoLyrics);
  const suspendHiddenVideo = useSettingsStore((state) => state.suspendHiddenVideo);
  const setSuspendHiddenVideo = useSettingsStore((state) => state.setSuspendHiddenVideo);

  return <SettingsSection title='Видео и тексты' beta>
    <SwitchSettings label='Показывать видео рядом с текстом'
      checked={separateVideoLyrics} onChange={setSeparateVideoLyrics} />
    <SwitchSettings label='Приостанавливать видео, когда окно скрыто'
      checked={suspendHiddenVideo} onChange={setSuspendHiddenVideo} />
    <LyricsSourceSettings />
  </SettingsSection>;
}

function LibrarySettings() {
  return <>
    <MediaFoldersSettings />
    <TrashSettings />
  </>;
}

function AppSettings() {
  return <SettingsSection title='Обновления и диагностика'>
    <div className='settings-rows'>
      <UpdateSettings />
      <DiagnosticsSettings />
    </div>
  </SettingsSection>;
}
