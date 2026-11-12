import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';

export const useTrackCover = (path: string | undefined) => {
  const [cover, setCover] = useState<{ path: string; src: string }>();

  useEffect(() => {
    if (!path) return;

    let active = true;
    let src: string | undefined;
    void invoke<ArrayBuffer>('get_cover', { path }).then((bytes) => {
      if (active && bytes.byteLength > 0) {
        src = URL.createObjectURL(new Blob([bytes]));
        setCover({ path, src });
      }
    }).catch(() => undefined);

    return () => {
      active = false;
      if (src) URL.revokeObjectURL(src);
    };
  }, [path]);

  return {
    coverSrc: cover && cover.path === path ? cover.src : undefined,
    clearCover: () => setCover(undefined),
  };
};
