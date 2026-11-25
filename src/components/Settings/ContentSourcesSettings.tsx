import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { SettingsSection } from '@/components/Settings/SettingsSection';
import { SelectSettings } from '@/components/Settings/SelectSettings';
import { SwitchSettings } from '@/components/Settings/SwitchSettings';
import { useSettingsStore } from '@/stores/settingsStore';
import type { LyricsSource, MetadataSource } from '@/types/Settings';
import { lyricaProviders } from '@/utils/lyrica';

export const metadataSources = [
  { value: 'auto', label: 'Автоматически' },
  { value: 'ommr', label: 'OMMR' },
  { value: 'musicBrainz', label: 'MusicBrainz' },
  { value: 'itunes', label: 'iTunes' },
  { value: 'genius', label: 'Genius' },
  { value: 'off', label: 'Только локальные теги' },
] as const;

export function CatalogSettings() {
  const metadataSource = useSettingsStore((state) => state.metadataSource);
  const showExternalAlbumTracks = useSettingsStore((state) => state.showExternalAlbumTracks);

  return <SettingsSection title='Внешние каталоги'>
    <div className='settings-rows'>
      <SelectSettings<MetadataSource> label='Основной источник метаданных' options={metadataSources}
        value={metadataSource} onChange={(metadataSource) => useSettingsStore.setState({ metadataSource })} />
      <SwitchSettings label='Показывать нескачанные треки в альбомах'
        checked={showExternalAlbumTracks} onChange={(showExternalAlbumTracks) => useSettingsStore.setState({ showExternalAlbumTracks })} />
      <CatalogStorageSettings />
    </div>
  </SettingsSection>;
}

const storageOptions = [
  { value: '64', label: '64 МБ' },
  { value: '256', label: '256 МБ' },
  { value: '512', label: '512 МБ' },
  { value: '1024', label: '1 ГБ' },
  { value: '2048', label: '2 ГБ' },
  { value: '4096', label: '4 ГБ' },
] as const;
type StorageStatus = { maxMib: number; usedBytes: number };
const formatStorage = new Intl.NumberFormat('ru', { maximumFractionDigits: 1 });

function CatalogStorageSettings() {
  const [status, setStatus] = useState<StorageStatus>();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState(false);
  useEffect(() => {
    let active = true;
    void invoke<StorageStatus>('library_catalog_storage')
      .then((data) => { if (active) setStatus(data); })
      .catch(() => { if (active) setError(true); });
    return () => { active = false; };
  }, []);

  const change = async (value: string) => {
    setPending(true);
    setError(false);
    try {
      setStatus(await invoke<StorageStatus>('library_catalog_storage', { maxMib: Number(value) }));
    } catch {
      setError(true);
    } finally {
      setPending(false);
    }
  };
  return <>
    <SelectSettings label='Лимит локального хранилища каталогов' options={storageOptions}
      value={String(status?.maxMib ?? 512)} disabled={pending || (!status && !error)}
      onChange={(value) => void change(value)} />
    {(pending || (!status && !error)) && <LoadingIndicator label='Проверка хранилища каталогов' />}
    {status && <p className='release-muted'>Занято {formatStorage.format(status.usedBytes / (1024 * 1024))} МБ</p>}
    {error && <p className='kit-error' role='alert'>Не удалось обновить настройки хранилища.</p>}
  </>;
}

export function LyricsSourceSettings() {
  const lyricsSource = useSettingsStore((state) => state.lyricsSource);

  return <SelectSettings<LyricsSource> label='Основной источник текста'
    options={[...lyricaProviders, { value: 'genius', label: 'Genius' }, { value: 'local', label: 'Локальный текст' }]}
    value={lyricsSource} onChange={(lyricsSource) => useSettingsStore.setState({ lyricsSource })} />;
}
