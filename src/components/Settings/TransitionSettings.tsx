import { useState } from 'preact/hooks';
import { Spinner } from '@/components/LoadingIndicator';
import { type SelectOption } from '@/components/Select';
import { SelectSettings } from '@/components/Settings/SelectSettings';
import { SwitchSettings } from '@/components/Settings/SwitchSettings';
import { Slider } from '@/components/Slider';
import { usePlayerStore } from '@/stores/playerStore';
import { TransitionWaveform, transitionPresets as presets, transitionSeconds as seconds } from '@/components/TransitionWaveform';
import { useSettingsStore } from '@/stores/settingsStore';
import { type TransitionSettings as Settings } from '@/types/Settings';

const modes: SelectOption<Settings['mode']>[] = [
  { value: 'smooth', label: 'Плавный' },
  { value: 'advanced', label: 'Продвинутый' },
];
const titleFromPath = (path: string) => path.replace(/^.*[\\/]/, '').replace(/\.[^.]+$/, '');

export function TransitionSettings() {
  const config = useSettingsStore((state) => state.transitions);
  const update = useSettingsStore((state) => state.setTransitions);

  return (
    <SwitchSettings label='Переходы между треками' beta checked={config.enabled}
      onChange={(enabled) => update({ enabled })}>
      <SelectSettings label='Режим перехода' options={modes} value={config.mode}
        onChange={(mode) => update({ mode })} />
      {config.mode === 'smooth' ? (
        <TransitionSlider label='Длительность перехода' value={config.seconds} min={1} max={12} step={0.5}
          format={seconds} onCommit={(seconds) => update({ seconds })} />
      ) : (
        <>
          <SelectSettings label='Пресет' options={presets} value={config.preset}
            onChange={(preset) => update({ preset })} />
          <TransitionSlider label='Максимальная длительность' value={config.maxSeconds} min={2} max={12} step={0.5}
            format={seconds} onCommit={(maxSeconds) => update({ maxSeconds })} />
          <TransitionSlider label='Сила фильтров' value={Math.round(config.intensity * 100)} min={0} max={100} step={5}
            format={(value) => `${value}%`} onCommit={(value) => update({ intensity: value / 100 })} />
          <SwitchSettings label='Сокращать тишину в начале и конце треков'
            checked={config.trimSilence} onChange={(trimSilence) => update({ trimSilence })} />
          <SwitchSettings label='Совмещать доли при близком темпе'
            checked={config.alignBeats} onChange={(alignBeats) => update({ alignBeats })} />
          <NextTransition />
        </>
      )}
    </SwitchSettings>
  );
}

function TransitionSlider({ label, value, min, max, step, format, onCommit }: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (value: number) => string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState<number | null>(null);
  const display = draft ?? value;
  return (
    <div className='settings-row'>
      <p>{label}</p>
      <div className='transition-slider'>
        <Slider label={label} value={display} min={min} max={max} step={step}
          onValueChange={setDraft} onValueCommit={(value) => { onCommit(value); setDraft(null); }} />
        <span>{format(display)}</span>
      </div>
    </div>
  );
}

function NextTransition() {
  const pair = usePlayerStore((state) => state.transition);
  const currentTrack = usePlayerStore((state) => state.currentTrack);
  const nextTrack = usePlayerStore((state) => state.nextTrack);
  const preset = presets.find(({ value }) => value === pair?.preset)?.label;

  return (
    <div className='transition-card'>
      <div className='transition-card-heading'>
        <h3>Следующий переход</h3>
        {pair && <span>{preset} · до {seconds(pair.seconds)}</span>}
      </div>
      {pair ? (
        <>
          <div className='transition-waveforms'>
            <TransitionWaveform label='Уходит' title={currentTrack?.name ?? titleFromPath(pair.fromPath)}
              side='tail' data={pair.tail} overlap={pair.seconds} offset={pair.trim} />
            <TransitionWaveform label='Вступает' title={nextTrack?.name ?? titleFromPath(pair.toPath)}
              side='head' data={pair.head} overlap={pair.seconds} offset={pair.cue} />
          </div>
          {pair.status === 'analyzing' && (
            <p className='transition-status' role='status'><Spinner size={14} /> Анализ фрагментов</p>
          )}
          {pair.status === 'waiting' && <p className='transition-status'>Анализ запускается при воспроизведении, ближе к концу трека.</p>}
          {pair.status === 'unavailable' && <p className='transition-status'>Анализ этих фрагментов недоступен.</p>}
        </>
      ) : <p className='transition-status'>Ожидает следующей пары аудиотреков.</p>}
    </div>
  );
}
