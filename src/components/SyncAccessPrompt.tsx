import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { setSyncServerAccess, startSyncServerAccess, useSyncServerAccess } from '@/stores/syncServerAccessStore';

export function SyncAccessPrompt() {
  const { devices, action } = useSyncServerAccess();
  const [dismissed, setDismissed] = useState<string[]>([]);
  const [error, setError] = useState('');
  const request = devices?.find((device) => device.status === 'pending' && !dismissed.includes(device.id));

  useEffect(startSyncServerAccess, []);
  useEffect(() => setError(''), [request?.id]);

  if (!request) return null;
  const decide = async (allowed: boolean) => {
    setError('');
    if (!await setSyncServerAccess(request.id, allowed)) {
      setError(useSyncServerAccess.getState().accessError ?? 'Не удалось обновить доступ.');
    }
  };

  return <Dialog key={request.id} open title='Разрешить подключение?'
    description={<>Устройство <strong>{request.name}</strong> запрашивает доступ к YAP Sync.</>}
    busy={Boolean(action)} onDismiss={() => setDismissed((ids) => [...ids, request.id])}>
    {error && <p className='kit-error' role='alert'>{error}</p>}
    <div className='kit-dialog-actions'>
      <Button data-initial-focus disabled={Boolean(action)} pending={action?.id === request.id && !action.allowed}
        onClick={() => void decide(false)}>Отклонить</Button>
      <Button variant='primary' disabled={Boolean(action)} pending={action?.id === request.id && action.allowed}
        onClick={() => void decide(true)}>Разрешить</Button>
    </div>
  </Dialog>;
}
