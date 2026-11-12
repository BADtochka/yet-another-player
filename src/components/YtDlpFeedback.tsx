import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { dismissGatekeeperDialog, openMacosPrivacySettings, useYtDlpStatus } from '@/utils/ytDlp';

export const YtDlpChecking = ({ label = 'Проверяем запуск yt-dlp…' }: { label?: string }) => (
  <span className='inline-flex items-center gap-2 text-sm text-(--yap-muted)' role='status'>
    <span className='size-3.5 animate-spin rounded-full border-2 border-current border-r-transparent motion-reduce:animate-none'
      aria-hidden='true' />
    {label}
  </span>
);

export const YtDlpGatekeeperDialog = () => {
  const open = useYtDlpStatus((state) => state.gatekeeperDialogOpen);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState(false);

  useEffect(() => { if (open) setOpenError(false); }, [open]);

  const openSettings = async () => {
    setOpening(true);
    setOpenError(false);
    try {
      await openMacosPrivacySettings();
      dismissGatekeeperDialog();
    } catch {
      setOpenError(true);
    } finally {
      setOpening(false);
    }
  };

  return (
    <Dialog open={open} title='macOS заблокировала yt-dlp' busy={opening} onDismiss={dismissGatekeeperDialog}
      description='В разделе «Конфиденциальность и безопасность» прокрутите до блока «Безопасность». Если вы доверяете этому файлу, нажмите «Всё равно открыть» рядом с yt-dlp и подтвердите действие. Затем вернитесь в YAP и повторите действие.'>
      {openError && <p role='alert' className='kit-error'>
        Откройте раздел вручную через меню Apple → «Системные настройки» → «Конфиденциальность и безопасность».
      </p>}
      <div className='kit-dialog-actions'>
        <Button variant='primary' icon='Open' disabled={opening} onClick={() => void openSettings()}>
          Открыть «Конфиденциальность и безопасность»
        </Button>
      </div>
    </Dialog>
  );
};
