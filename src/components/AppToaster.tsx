import { Toaster } from 'sonner';
import 'sonner/dist/styles.css';
import { Icon } from '@/components/Icons/Icon';

export const AppToaster = () => (
  <Toaster theme='dark' position='bottom-center' offset={126} closeButton style={{ '--width': '400px' }}
    containerAriaLabel='Уведомления'
    icons={{ close: <Icon name='X' size={16} /> }}
    toastOptions={{
      closeButtonAriaLabel: 'Закрыть уведомление',
      classNames: {
        toast: 'yap-toast',
        title: 'yap-toast-title',
        description: 'yap-toast-description',
        actionButton: 'yap-toast-action',
        cancelButton: 'yap-toast-cancel',
        closeButton: 'yap-toast-close',
      },
    }}
  />
);
