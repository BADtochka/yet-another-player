import { Toaster } from 'sonner';
import 'sonner/dist/styles.css';

export const AppToaster = () => (
  <Toaster
    theme='dark'
    position='bottom-right'
    closeButton
    toastOptions={{
      style: {
        background: '#1a1d26',
        border: '1px solid #2a2d36',
        color: '#d9d9d9',
      },
      classNames: {
        description: 'text-[#8d8d8d]',
        actionButton: 'bg-[#d9d9d9] text-[#01040c] font-medium',
        cancelButton: 'bg-[#2a2d36] text-[#d9d9d9]',
      },
    }}
  />
);
