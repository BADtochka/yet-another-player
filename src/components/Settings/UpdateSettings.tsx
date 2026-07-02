import { getVersion } from '@tauri-apps/api/app';
import { useEffect, useState } from 'preact/hooks';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { SettingsTextAction } from '@/components/Settings/SettingsTextAction';
import { checkForAppUpdatesManually } from '@/utils/checkForUpdates';

export const UpdateSettings = () => {
  const [version, setVersion] = useState<string | null>(null);
  const [isChecking, setIsChecking] = useState(false);

  useEffect(() => {
    void getVersion()
      .then((appVersion) => setVersion(appVersion))
      .catch(() => setVersion(null));
  }, []);

  const onCheckUpdates = async () => {
    setIsChecking(true);

    try {
      await checkForAppUpdatesManually();
    } finally {
      setIsChecking(false);
    }
  };

  return (
    <SettingsSection title='О приложении'>
      <div className='flex flex-col gap-4 p-2.5'>
        <div className='flex items-center justify-between gap-5'>
          <div className='flex min-w-0 flex-col gap-1'>
            <p className='text-base font-medium text-[#d9d9d9]'>Версия</p>
            <p className='text-sm font-medium text-[#8d8d8d]'>Проверить наличие обновлений на GitHub Releases</p>
          </div>

          <SettingsTextAction disabled={isChecking || version === null} onClick={onCheckUpdates}>
            {isChecking ? 'Проверяю...' : version ? `v${version}` : '...'}
          </SettingsTextAction>
        </div>
      </div>
    </SettingsSection>
  );
};
