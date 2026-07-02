import { useEffect, useState } from 'preact/hooks';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { SettingsTextAction } from '@/components/Settings/SettingsTextAction';
import { type AudioEnvironment, getAudioEnvironment } from '@/utils/audioEnvironment';
import { logError, openConfigFolder } from '@/utils/appLogs';

export const DiagnosticsSettings = () => {
  const [isOpening, setIsOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [audioEnvironment, setAudioEnvironment] = useState<AudioEnvironment | null>(null);

  useEffect(() => {
    void getAudioEnvironment().then(setAudioEnvironment).catch(() => {
      setAudioEnvironment(null);
    });
  }, []);

  const onOpenConfigFolder = async () => {
    setError(null);
    setIsOpening(true);

    try {
      await openConfigFolder();
    } catch (openError) {
      setError(openError instanceof Error ? openError.message : String(openError));
      logError('Failed to open config folder', openError);
    } finally {
      setIsOpening(false);
    }
  };

  return (
    <SettingsSection title='Диагностика'>
      <div className='flex flex-col gap-4 p-2.5'>
        <div className='flex items-center justify-between gap-5'>
          <div className='flex min-w-0 flex-col gap-1'>
            <p className='text-base font-medium text-[#d9d9d9]'>Папка конфигов и логов</p>
            <p className='text-sm font-medium text-[#8d8d8d]'>Логи пишутся в logs/yap.log внутри папки конфигов</p>
          </div>

          <SettingsTextAction disabled={isOpening} onClick={onOpenConfigFolder}>
            {isOpening ? 'Открываю...' : 'Открыть'}
          </SettingsTextAction>
        </div>

        {audioEnvironment && (
          <div className='flex flex-col gap-2 rounded-xl border border-[#101C3A] bg-[#01040c] p-3'>
            <p className='text-sm font-medium text-[#d9d9d9]'>Аудио окружение</p>
            <p className='text-sm text-[#8d8d8d]'>
              Платформа: {audioEnvironment.platform}
              {audioEnvironment.isWsl ? ' (WSL)' : ''}
            </p>
            <p className='text-sm text-[#8d8d8d]'>
              PulseAudio: {audioEnvironment.pulseServer ?? 'не настроен'}
            </p>
            <p className='text-sm text-[#8d8d8d]'>
              Буфер: {audioEnvironment.pulseLatencyMs ? `${audioEnvironment.pulseLatencyMs} ms` : 'по умолчанию'}
            </p>
            <p className='text-sm text-[#8d8d8d]'>
              Выравнивание: ReplayGain из тегов MP3 через `audio.volume`
            </p>
            <p className='text-sm text-[#8d8d8d]'>
              Web Audio: не используется
            </p>
            {audioEnvironment.notes.length > 0 && (
              <ul className='list-disc pl-5 text-sm text-[#8d8d8d]'>
                {audioEnvironment.notes.map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            )}
          </div>
        )}

        {error && <p className='text-sm text-[#da3e3e]'>{error}</p>}
      </div>
    </SettingsSection>
  );
};
