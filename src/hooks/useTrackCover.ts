import { invoke } from '@tauri-apps/api/core';
import { useCatalogRevision } from '@/hooks/useLibrary';
import { useEffect, useState } from 'preact/hooks';

export const useTrackCover = (path: string | undefined) => {
  const [cover, setCover] = useState<{ path: string; src: string; revision: number }>();

  const revision = useCatalogRevision((state) => state.revision);

  useEffect(() => {
    if (!path) return;

    let active = true;
    let src: string | undefined;
    void invoke<ArrayBuffer | number[]>('get_cover', { path }).then(async (result) => {
      const bytes = Array.isArray(result) ? Uint8Array.from(result) : result;
      if (active && bytes.byteLength > 0) {
        src = URL.createObjectURL(new Blob([bytes]));
        setCover({ path, src, revision });
      } else if (active) {
        const url = await invoke<string | null>('get_resolver_cover', { path });
        if (active && url) setCover({ path, src: url, revision });
      }
    }).catch(() => undefined);

    return () => {
      active = false;
      if (src) URL.revokeObjectURL(src);
    };
  }, [path, revision]);

  return {
    coverSrc: cover && cover.path === path && cover.revision === revision ? cover.src : undefined,
    clearCover: () => setCover(undefined),
  };
};
