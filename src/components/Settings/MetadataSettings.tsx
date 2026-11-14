import { invoke } from '@tauri-apps/api/core';
import { useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { Input } from '@/components/Input';
import { Select, type SelectOption } from '@/components/Select';
import { useSettingsStore } from '@/stores';
import type { ResolverMode } from '@/types/Settings';

const modes: SelectOption<ResolverMode>[] = [
  { value: 'off', label: 'Выключен' },
  { value: 'local', label: 'Локальный сервер' },
  { value: 'remote', label: 'Внешний сервер' },
];

const providers = [
  ['applemusic', 'Apple Music'],
  ['deezer', 'Deezer'],
  ['musicbrainz', 'MusicBrainz'],
  [['yt', 'music'].join(''), 'Музыкальный каталог'],
] as const;

export function MetadataSettings({ installed, onDismiss }: {
  installed: boolean;
  onDismiss: () => void;
}) {
  const { resolverMode, resolverUrl, resolverSources, setResolver } = useSettingsStore();
  const [mode, setMode] = useState<ResolverMode>(resolverMode ?? (resolverUrl ? 'remote' : 'off'));
  const [url, setUrl] = useState(resolverUrl);
  const [sources, setSources] = useState(resolverSources);
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<{ error?: string }>();
  let validUrl = false;
  try {
    const address = new URL(url.trim());
    validUrl = ['http:', 'https:'].includes(address.protocol) && !address.username && !address.password
      && !address.search && !address.hash;
  } catch {
    // The field can be incomplete while the user edits it.
  }
  const valid = mode !== 'remote' || validUrl;

  async function checkConnection() {
    setChecking(true);
    setResult(undefined);
    try {
      await invoke('test_ommr', { config: { mode, url: url.trim() || null, sources } });
      setResult({});
    } catch (error) {
      setResult({ error: String(error) });
    } finally {
      setChecking(false);
    }
  }

  return <Dialog open title='Настройка OMMR' busy={checking} onDismiss={onDismiss}>
    <form className='kit-dialog-form' onSubmit={(event) => {
      event.preventDefault();
      if (!valid || checking) return;
      setResolver({ resolverMode: mode, resolverUrl: url, resolverSources: sources });
      onDismiss();
    }}>
      <Select label='Сервер OMMR' value={mode} options={modes} disabled={checking}
        onChange={(value) => { setMode(value); setResult(undefined); }} />
      {mode === 'local' && !installed && <p>Установите OMMR, чтобы использовать локальный сервер.</p>}
      {mode === 'remote' && <Input type='url' aria-label='Адрес сервера OMMR' aria-invalid={!!url && !validUrl}
        disabled={checking} placeholder='https://ommr.example.org' value={url}
        onInput={(event) => { setUrl(event.currentTarget.value); setResult(undefined); }} />}
      {mode !== 'off' && <fieldset className='grid gap-2' disabled={checking}>
        <legend>Источники метаданных</legend>
        <p className='settings-version'>Если ничего не выбрано, используются настройки сервера.</p>
        {providers.map(([value, name]) => <label key={value} className='flex items-center gap-2'>
          <input type='checkbox' checked={sources.includes(value)} onChange={(event) => {
            const checked = event.currentTarget.checked;
            setSources((current) => checked ? [...current, value] : current.filter((source) => source !== value));
            setResult(undefined);
          }} />
          {name}
        </label>)}
      </fieldset>}
      {result && <p role={result.error ? 'alert' : 'status'} className={result.error ? 'kit-error' : undefined}>
        {result.error ?? 'Сервер OMMR доступен.'}
      </p>}
      <div className='kit-dialog-actions'>
        {mode !== 'off' && <Button size='small' icon='Refresh' pending={checking}
          disabled={!valid || (mode === 'local' && !installed)} onClick={() => void checkConnection()}>
          Проверить подключение
        </Button>}
        <Button type='submit' size='small' variant='primary' disabled={!valid || checking}>Сохранить</Button>
      </div>
    </form>
  </Dialog>;
}
