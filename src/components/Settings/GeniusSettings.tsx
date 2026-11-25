import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useId, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Input } from '@/components/Input';
import { Icon } from '@/components/Icons/Icon';
import { Switch } from '@/components/Switch';
import { useSettingsStore } from '@/stores/settingsStore';
import { setExternalToolEnabled } from '@/utils/externalTools';
import { geniusDisconnect, geniusError, geniusIsConfigured, geniusSaveToken } from '@/utils/genius';

const CLIENTS_URL = 'https://genius.com/api-clients/new';
const DOCS_URL = 'https://docs.genius.com/';

export function GeniusSettings() {
  const [configured, setConfigured] = useState<boolean>();
  const disabled = useSettingsStore((state) => state.disabledTools.includes('genius'));
  const [toggling, setToggling] = useState(false);
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState<'save' | 'disconnect'>();
  const [token, setToken] = useState('');
  const [statusError, setStatusError] = useState<string>();
  const [error, setError] = useState<string>();
  const inputId = useId();
  const errorId = useId();

  useEffect(() => {
    let active = true;
    void geniusIsConfigured().then((value) => {
      if (active) setConfigured(value);
    }).catch((reason) => {
      if (active) setStatusError(geniusError(reason));
    });
    return () => { active = false; };
  }, []);

  function dismiss() {
    setToken('');
    setError(undefined);
    setOpen(false);
  }

  async function update(action: 'save' | 'disconnect') {
    if (pending || (action === 'save' && !token.trim())) return;
    setPending(action);
    setError(undefined);
    try {
      if (action === 'save') await geniusSaveToken(token);
      else await geniusDisconnect();
      setConfigured(action === 'save');
      useSettingsStore.setState({ geniusConfigured: action === 'save' });
      setStatusError(undefined);
      dismiss();
    } catch (reason) {
      setError(geniusError(reason));
    } finally {
      setPending(undefined);
    }
  }

  async function toggle(enabled: boolean) {
    setToggling(true);
    setStatusError(undefined);
    try {
      await setExternalToolEnabled('genius', enabled);
    } catch (reason) {
      setStatusError(String(reason));
    } finally {
      setToggling(false);
    }
  }

  return (
    <div className='settings-tool'>
      <div className='settings-row'>
        <div className='settings-row-info'>
          <h4><button type='button' className='settings-source' aria-label='Документация Genius API'
            onClick={() => void openUrl(DOCS_URL).catch(() => setStatusError('Не удалось открыть документацию Genius.'))}>
            Genius <Icon name='Open' size={16} />
          </button></h4>
          <p>Поиск текстов песен и метаданных по токену</p>
        </div>
        <div className='settings-row-actions'>
          {configured === undefined && !statusError && <LoadingIndicator label='Проверка подключения Genius' />}
          {configured
            ? <Button size='icon' icon='Settings' aria-label='Настроить Genius' title='Настроить Genius' onClick={() => setOpen(true)} />
            : <Button variant='primary' disabled={configured === undefined && !statusError} onClick={() => setOpen(true)}>
              Подключить
            </Button>}
          {toggling && <LoadingIndicator label='Изменение состояния Genius' />}
          {configured && <Switch label='Использовать Genius' checked={!disabled}
            onChange={(enabled) => void toggle(enabled)} disabled={toggling || Boolean(pending)} />}
        </div>
      </div>
      {statusError && <p className='kit-error' role='alert'>{statusError}</p>}
      <Dialog open={open} title='Настройка Genius' busy={Boolean(pending)} onDismiss={dismiss}
        description={configured ? 'Токен сохранён в системном хранилище. Его можно заменить или удалить.' : 'Подключите свой аккаунт Genius, чтобы искать песни из YAP.'}>
        {!configured && (
          <div className='genius-instructions'>
            <ol>
              <li>Войдите в Genius и откройте страницу создания клиента.</li>
              <li>Укажите название клиента и адрес приложения, если Genius его запросит. Для поиска Redirect URI не нужен.</li>
              <li>Создайте клиент, затем нажмите <strong>Generate Access Token</strong> и скопируйте <strong>Client Access Token</strong>.</li>
            </ol>
            <Button icon='Open' size='small' onClick={() => void openUrl(CLIENTS_URL).catch(() => setError('Не удалось открыть страницу Genius.'))}>
              Создать клиент Genius
            </Button>
          </div>
        )}
        <form className='kit-dialog-form' onSubmit={(event) => { event.preventDefault(); void update('save'); }}>
          <div className='genius-token-field'>
            <label htmlFor={inputId}>Client Access Token</label>
            <Input id={inputId} type='password' autoComplete='off' spellcheck={false} maxLength={2048}
              data-initial-focus aria-describedby={error ? errorId : undefined}
              value={token} placeholder={configured ? 'Введите новый токен' : 'Вставьте токен'}
              onInput={(event) => setToken(event.currentTarget.value)} disabled={Boolean(pending)} />
          </div>
          {error && <p id={errorId} className='kit-error' role='alert'>{error}</p>}
          <div className='kit-dialog-actions'>
            {configured && <Button variant='danger' pending={pending === 'disconnect'} disabled={Boolean(pending)}
              onClick={() => void update('disconnect')}>Удалить токен</Button>}
            <Button type='submit' variant='primary' pending={pending === 'save'} disabled={Boolean(pending) || !token.trim()}>
              {configured ? 'Сохранить' : 'Подключить'}
            </Button>
          </div>
        </form>
      </Dialog>
    </div>
  );
}
