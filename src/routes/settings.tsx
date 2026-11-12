import { SettingsSection } from '@/components/Settings/SettingsSection';
import { DiagnosticsSettings } from '@/components/Settings/DiagnosticsSettings';
import { ExternalToolsSettings } from '@/components/Settings/ExternalToolsSettings';
import { MediaFoldersSettings } from '@/components/Settings/MediaFoldersSettings';
import { SelectSettings } from '@/components/Settings/SelectSettings';
import { SwitchSettings } from '@/components/Settings/SwitchSettings';
import { UpdateSettings } from '@/components/Settings/UpdateSettings';
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
    <div className='settings-page'>
      <MediaFoldersSettings />
      <ExternalToolsSettings />
      <SettingsSection title='Прочее' description='Версия, конфигурация и логи'>
        <div className='settings-rows'>
          <UpdateSettings />
          <DiagnosticsSettings />
        </div>
      </SettingsSection>
      <SettingsSection title='Воспроизведение'>
        <SwitchSettings label='Установить одну громкость для всех треков'
          checked={volumeNormEnabled} onChange={setVolumeNormEnabled}>
          <SelectSettings label='Уровень громкости' options={[
            { label: 'Тихо', value: 'quiet' },
            { label: 'Нормально', value: 'normal' },
            { label: 'Громко', value: 'loud' },
          ]} value={volumeNormLevel} onChange={setVolumeNormLevel} />
        </SwitchSettings>
      </SettingsSection>
    </div>
  );
}
