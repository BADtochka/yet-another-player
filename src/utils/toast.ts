import { toast as sonnerToast, type ExternalToast } from 'sonner';

const serializeError = (error: unknown) => {
  if (error instanceof Error) {
    return error.message;
  }

  return String(error);
};

export const toast = {
  info(message: string, options?: ExternalToast) {
    sonnerToast.info(message, options);
  },

  error(message: string, error?: unknown, options?: ExternalToast) {
    sonnerToast.error(message, {
      ...options,
      description: error === undefined ? options?.description : serializeError(error),
    });
  },

  loading(message: string, options?: ExternalToast) {
    return sonnerToast.loading(message, options);
  },

  message(message: string, options?: ExternalToast) {
    sonnerToast(message, options);
  },

  dismiss: sonnerToast.dismiss,
};
