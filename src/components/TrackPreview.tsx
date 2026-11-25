import { useEffect, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { SeekBar } from '@/components/SeekBar';
import { SliderFrame } from '@/components/Slider';
import { closePreview, preparePreview, seekPreview, setPreviewPlaying, usePlayerStore } from '@/stores/playerStore';
import { formatPlaybackTime } from '@/utils/formatPlaybackTime';
import { startMusicPreview, YtDlpError, type MusicSearchResult } from '@/utils/ytDlp';

export function TrackPreview({ result, onDismiss }: { result: MusicSearchResult; onDismiss: () => void }) {
  const [id, setId] = useState<number>();
  const [error, setError] = useState<string>();
  const [drag, setDrag] = useState<number>();
  const preview = usePlayerStore((state) => state.preview?.id === id ? state.preview : null);
  useEffect(() => {
    let active = true;
    let previewId: number | undefined;
    void (async () => {
      try {
        previewId = await preparePreview(result.title, result.artist ?? '');
        if (!active) {
          closePreview(previewId);
          return;
        }
        setId(previewId);
        await startMusicPreview(previewId, result.id);
      } catch (reason) {
        if (active) setError(reason instanceof YtDlpError ? reason.message : 'Не удалось открыть предпросмотр.');
      }
    })();
    return () => {
      active = false;
      if (previewId !== undefined) closePreview(previewId);
    };
  }, [result.id, result.title, result.artist]);

  const duration = preview?.duration ?? 0;
  const position = drag ?? preview?.position ?? 0;
  return <Dialog open title='Предпросмотр' onDismiss={onDismiss}>
    <div>
      <p className='font-semibold'>{result.title}</p>
      {result.artist && <p>{result.artist}</p>}
    </div>
    {error ? <p className='kit-error' role='alert'>{error}</p>
      : preview && !preview.loading ? <>
        <div className='flex justify-center'>
          <Button variant='ghost' size='icon' icon={preview.playing ? 'Pause' : 'Play'}
            aria-label={preview.playing ? 'Пауза' : 'Воспроизвести'}
            onClick={() => setPreviewPlaying(preview.id, !preview.playing)} />
        </div>
        <SliderFrame>
          <span>{formatPlaybackTime(position)}</span>
          <SeekBar className='flex-1' label='Позиция предпросмотра' value={position} max={Math.max(0.1, duration)}
            disabled={duration <= 0} onSeek={setDrag} onSeekEnd={(time) => {
              setDrag(undefined);
              seekPreview(preview.id, time);
            }} />
          <span>{formatPlaybackTime(duration)}</span>
        </SliderFrame>
      </> : id !== undefined && !preview
        ? <p role='status'>Предпросмотр завершён.</p>
        : <LoadingIndicator label='Загрузка предпросмотра' />}
  </Dialog>;
}
