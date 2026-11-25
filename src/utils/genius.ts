import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '@/stores/settingsStore';

export async function geniusIsConfigured() {
  const geniusConfigured = await invoke<boolean>('genius_is_configured');
  useSettingsStore.setState({ geniusConfigured });
  return geniusConfigured;
}
export const geniusSaveToken = (token: string) => invoke<void>('genius_save_token', { token });
export const geniusDisconnect = () => invoke<void>('genius_disconnect');
export const geniusLyrics = (title: string, artists: string[]) =>
  invoke<string | null>('genius_lyrics', { title, artists });

const messages: Record<string, string> = {
  disabled: 'Genius отключён в настройках инструментов.',
  invalidToken: 'Проверьте Client Access Token: в нём не должно быть пробелов.',
  notConfigured: 'Сначала подключите Genius в настройках.',
  credentialStore: 'Не удалось открыть системное хранилище паролей. Проверьте, что оно доступно и разблокировано.',
  unauthorized: 'Genius отклонил токен. Скопируйте Client Access Token из настроек клиента Genius.',
  rateLimited: 'Genius ограничил число запросов. Попробуйте чуть позже.',
  network: 'Не удалось связаться с Genius. Проверьте интернет и попробуйте снова.',
  service: 'Genius не ответил как ожидается. Попробуйте чуть позже.',
  emptyQuery: 'Для поиска нужны название трека и исполнитель.',
};

export function geniusPageUrl(error: unknown) {
  if (!error || typeof error !== 'object' || !('pageUnavailable' in error)) return undefined;
  const page = error.pageUnavailable;
  if (!page || typeof page !== 'object' || !('url' in page) || typeof page.url !== 'string') return undefined;
  try {
    const url = new URL(page.url);
    if (url.origin === 'https://genius.com' && !url.username && !url.password && url.pathname.endsWith('-lyrics')) return url.href;
  } catch {
    return undefined;
  }
}

export function geniusError(error: unknown) {
  if (geniusPageUrl(error)) return 'Genius нашёл песню по токену, но ограничил загрузку текста в YAP.';
  return typeof error === 'string' && Object.prototype.hasOwnProperty.call(messages, error)
    ? messages[error] : 'Не удалось выполнить действие Genius.';
}
