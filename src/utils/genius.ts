import { invoke } from '@tauri-apps/api/core';

export interface GeniusSong {
  title: string;
  artist: string;
  url: string;
}

export const geniusIsConfigured = () => invoke<boolean>('genius_is_configured');
export const geniusSaveToken = (token: string) => invoke<void>('genius_save_token', { token });
export const geniusDisconnect = () => invoke<void>('genius_disconnect');
export const geniusSearch = (title: string, artist: string) =>
  invoke<GeniusSong[]>('genius_search', { title, artist });

const messages: Record<string, string> = {
  invalidToken: 'Проверьте Client Access Token: в нём не должно быть пробелов.',
  notConfigured: 'Сначала подключите Genius в настройках.',
  credentialStore: 'Не удалось открыть системное хранилище паролей. Проверьте, что оно доступно и разблокировано.',
  unauthorized: 'Genius отклонил токен. Скопируйте Client Access Token из настроек клиента Genius.',
  rateLimited: 'Genius ограничил число запросов. Попробуйте чуть позже.',
  network: 'Не удалось связаться с Genius. Проверьте интернет и попробуйте снова.',
  service: 'Genius не ответил как ожидается. Попробуйте чуть позже.',
  emptyQuery: 'У этого трека нет названия для поиска.',
};

export const geniusError = (error: unknown) =>
  typeof error === 'string' && Object.prototype.hasOwnProperty.call(messages, error)
    ? messages[error] : 'Не удалось выполнить действие Genius.';
