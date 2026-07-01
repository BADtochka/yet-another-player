import { MediaFoldersSettings } from '@/components/Settings/MediaFoldersSettings';
import { SelectSettings } from '@/components/Settings/SelectSettings';
import { SwitchSettings } from '@/components/Settings/SwitchSettings';
import { useSettingsStore } from '@/stores';
import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/settings')({
  component: RouteComponent,
});

function RouteComponent() {
  const volumeNormEnabled = useSettingsStore((state) => state.volumeNormEnabled);
  const setVolumeNormEnabled = useSettingsStore((state) => state.setVolumeNormEnabled);
  const volumeNormLevel = useSettingsStore((state) => state.volumeNormLevel);
  const setVolumeNormLevel = useSettingsStore((state) => state.setVolumeNormLevel);

  return (
    <div className='mx-auto flex w-full max-w-[952px] flex-col gap-5'>
      <MediaFoldersSettings />
      <SwitchSettings
        label='Установить одну громкость для всех треков'
        checked={volumeNormEnabled}
        onChange={setVolumeNormEnabled}
      >
        <SelectSettings
          label='Установи желаемый уровень громкости в зависимости
          от окружающей обстановки'
          options={[
            {
              label: 'Тихо',
              value: 'quiet',
            },
            {
              label: 'Нормально',
              value: 'normal',
            },
            {
              label: 'Громко',
              value: 'loud',
            },
          ]}
          value={volumeNormLevel}
          onChange={setVolumeNormLevel}
        />
      </SwitchSettings>
    </div>
  );
}
