import type { SelectOption } from '@/components/Select';
import type { TransitionWaveform as WaveformData } from '@/stores/playerStore';
import type { TransitionPreset } from '@/types/Settings';

export const transitionPresets: SelectOption<TransitionPreset>[] = [
  { value: 'auto', label: 'Автоподбор' },
  { value: 'bassSwap', label: 'Смена баса' },
  { value: 'outro', label: 'Естественное затухание' },
  { value: 'hook', label: 'Вход на акцент' },
  { value: 'blend', label: 'Равномощное сведение' },
  { value: 'linear', label: 'Линейное затухание' },
  { value: 'dip', label: 'Приглушение в середине' },
  { value: 'rise', label: 'Плавное нарастание' },
  { value: 'lowPass', label: 'Фильтр низких частот' },
  { value: 'highPass', label: 'Фильтр высоких частот' },
  { value: 'cut', label: 'Быстрая склейка' },
];

const seconds = new Intl.NumberFormat('ru', { maximumFractionDigits: 1 });
export const transitionSeconds = (value: number) => `${seconds.format(value)} с`;

export function TransitionWaveform({ label, title, side, data, overlap, offset, maxCue, onCue }: {
  label: string;
  title: string;
  side: 'head' | 'tail';
  data: WaveformData | null;
  overlap: number;
  offset: number;
  maxCue?: number;
  onCue?: (seconds: number) => void;
}) {
  const windowSeconds = data?.windowSeconds ?? 20;
  const start = Math.min(100, (offset / windowSeconds) * 100);
  const width = Math.min(100 - start, (overlap / windowSeconds) * 100);
  const head = side === 'head';
  const maxHeadCue = maxCue ?? Math.max(0, windowSeconds - overlap);
  const cue = head ? Math.min(offset, maxHeadCue) : Math.max(0, windowSeconds - offset);
  const cueLabel = head ? 'Точка входа' : 'Точка выхода';
  const marker = head ? data?.hook : data?.fadeSeconds == null ? null : windowSeconds - data.fadeSeconds - data.silence;

  return (
    <div className='transition-waveform' data-side={side}>
      <div className='transition-waveform-caption'>
        <span>{label}</span>
        {data?.bpm && <span>≈ {data.bpm} BPM</span>}
      </div>
      <strong title={title}>{title}</strong>
      <div className='transition-bars'>
        <div className='transition-overlap' aria-hidden='true'
          style={{ insetInlineStart: `${head ? start : 100 - start - width}%`, width: `${width}%` }} />
        {marker != null && <span className='transition-signal-marker' aria-hidden='true'
          style={{ insetInlineStart: `${Math.max(0, marker / windowSeconds * 100)}%` }} />}
        {data?.waveform.map((amplitude, index) =>
          <i key={index} aria-hidden='true' style={{ height: `${Math.max(3, amplitude)}%` }} />)}
        {onCue && <input className='transition-cue-input' type='range' aria-label={cueLabel} step='0.01'
          min={head ? 0 : Math.min(windowSeconds, overlap)} max={head ? maxHeadCue : windowSeconds}
          aria-valuetext={transitionSeconds(head ? offset : Math.max(0, (data?.duration ?? windowSeconds) - offset))}
          value={cue} onInput={(event) => onCue(head ? event.currentTarget.valueAsNumber : windowSeconds - event.currentTarget.valueAsNumber)} />}
      </div>
      {marker != null && <small className='transition-signal-label'>{head
        ? `Акцент на ${transitionSeconds(marker)}`
        : `Затухание около ${transitionSeconds(data?.fadeSeconds ?? 0)}`}</small>}
    </div>
  );
}
