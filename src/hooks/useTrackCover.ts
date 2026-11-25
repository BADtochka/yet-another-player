import { invoke } from '@tauri-apps/api/core';
import { useCatalogRevision } from '@/hooks/useLibrary';
import { useSettingsStore } from '@/stores/settingsStore';
import type { MetadataSource } from '@/types/Settings';
import { useEffect, useMemo, useState } from 'preact/hooks';

function useCover(key: string | number | undefined, revision: number, primary?: MetadataSource, albumId?: string) {
  const requestKey = useMemo(() => ({ key, revision, primary, albumId }), [key, revision, primary, albumId]);
  const [cover, setCover] = useState<{ requestKey: typeof requestKey; src?: string }>();

  useEffect(() => {
    if (key === undefined) return;

    let active = true;
    let src: string | undefined;
    const request = primary !== undefined
      ? invoke<ArrayBuffer | number[]>('library_album_artwork_image', { url: key, primary, albumId })
      : typeof key === 'number'
        ? invoke<ArrayBuffer | number[]>('library_trash_cover', { id: key })
        : invoke<ArrayBuffer | number[]>('get_cover', { path: key });
    void request.then((result) => {
      const bytes = new Uint8Array(result instanceof ArrayBuffer ? result : Uint8Array.from(result).buffer);
      if (!active) return;
      if (bytes.length) src = URL.createObjectURL(new Blob([bytes]));
      setCover({ requestKey, src });
    }).catch(() => {
      if (active) setCover({ requestKey });
    });

    return () => {
      active = false;
      if (src) URL.revokeObjectURL(src);
    };
  }, [requestKey]);

  const current = cover?.requestKey === requestKey ? cover : undefined;
  return {
    coverSrc: current?.src,
    loading: key !== undefined && current === undefined,
    clearCover: () => setCover((previous) => previous && { ...previous, src: undefined }),
  };
}

export function useTrackCover(path: string | undefined) {
  return useCover(path || undefined, useCatalogRevision((state) => state.revision));
}

export function useTrashCover(id: number | undefined) {
  return useCover(id, 0);
}

export function useCatalogCover(url: string | undefined, albumId?: string, savedWhenOff = false) {
  const primary = useSettingsStore((state) => state.metadataSource);
  return useCover(primary === 'off' && !savedWhenOff ? undefined : url, 0, primary, albumId);
}
