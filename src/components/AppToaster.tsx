import { Toaster } from 'sonner';
import 'sonner/dist/styles.css';

export const AppToaster = () => (
  <Toaster
    theme='dark'
    position='bottom-right'
    offset={124}
    closeButton
    toastOptions={{
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
