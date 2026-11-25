import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { LoadingIndicator } from '@/components/LoadingIndicator';
import { Progress } from '@/components/Progress';
import { SelectSettings } from '@/components/Settings/SelectSettings';
import { SwitchSettings } from '@/components/Settings/SwitchSettings';
import { Slider } from '@/components/Slider';
import { TransitionWaveform, transitionPresets, transitionSeconds } from '@/components/TransitionWaveform';
import { closePreview, preparePreview, startPlaylistTransitionPreview, usePlayerStore, type TransitionWaveform as Waveform } from '@/stores/playerStore';
import { changePlaylists, type Playlist } from '@/stores/playlistStore';
import type { PlaylistTransition } from '@/types/Settings';
import type { Track } from '@/types/Track';

type PairAnalysis = { suggested: PlaylistTransition; tail: Waveform | null; head: Waveform | null };
const fallback: PlaylistTransition = { preset: 'blend', seconds: 5, trim: 0, cue: 0, intensity: 0.7 };

export function PlaylistTransitionEditor({ playlist, tracks, onClose }: {
  playlist: Playlist;
  tracks: Track[];
  onClose: () => void;
}) {
  const [pairId, setPairId] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const pairs = tracks.slice(0, -1).map((outgoing, index) => {
    const incoming = tracks[index + 1];
    return { outgoing, incoming, value: String(outgoing.id), label: `${outgoing.name} → ${incoming.name}` };
  });
  const pair = pairs.find((item) => item.value === pairId) ?? pairs[0];

  const setMix = async (enabled: boolean) => {
    setBusy(true);
    setError('');
    try {
      await changePlaylists({ type: 'setMix', id: playlist.id, enabled });
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };

  return <Dialog open title='Переходы в плейлисте' description={playlist.name}
    className='playlist-mix-dialog' busy={busy} onDismiss={onClose}>
    <SwitchSettings label='Сводить треки в плейлисте' checked={playlist.mixEnabled}
      onChange={(enabled) => { if (!busy) void setMix(enabled); }} />
    {busy && <LoadingIndicator label='Сохранение режима плейлиста' />}
    {error && <p className='kit-error' role='alert'>{error}</p>}
    {pair
      ? <>
        <SelectSettings label='Пара треков' className='playlist-pair-select' options={pairs}
          value={pair.value} onChange={setPairId} />
        <PairEditor key={`${pair.outgoing.id}:${pair.incoming.id}`} playlist={playlist}
          outgoing={pair.outgoing} incoming={pair.incoming} />
      </>
      : <p className='kit-message'>Добавьте хотя бы два трека, чтобы настроить переход.</p>}
  </Dialog>;
}

function PairEditor({ playlist, outgoing, incoming }: {
  playlist: Playlist;
  outgoing: Track;
  incoming: Track;
}) {
  const saved = playlist.transitions.find((entry) => entry.outgoingId === outgoing.id && entry.incomingId === incoming.id)?.settings;
  const [analysis, setAnalysis] = useState<PairAnalysis>();
  const [analyzing, setAnalyzing] = useState(true);
  const [analysisError, setAnalysisError] = useState(false);
  const [draft, setDraft] = useState<PlaylistTransition | null>(saved ?? null);
  const [busy, setBusy] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState('');
  const previewId = useRef<number>();
  const nativePreview = usePlayerStore((state) => state.preview);
  const previewing = previewId.current !== undefined && previewId.current === nativePreview?.id;
  const request = { id: playlist.id, outgoingId: outgoing.id, incomingId: incoming.id };
  const dirty = JSON.stringify(saved ?? null) !== JSON.stringify(draft);

  useEffect(() => {
    let active = true;
    const timer = window.setTimeout(() => void invoke<PairAnalysis>('library_playlist_transition_analysis', request)
      .then((data) => { if (active) setAnalysis(data); })
      .catch(() => { if (active) setAnalysisError(true); })
      .finally(() => { if (active) setAnalyzing(false); }), 160);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [playlist.id, outgoing.id, incoming.id]);

  const live = useRef(true);
  useEffect(() => () => {
    live.current = false;
    if (previewId.current !== undefined) closePreview(previewId.current);
  }, []);

  const mix = draft ?? analysis?.suggested ?? fallback;
  const bounds = {
    tail: analysis?.tail?.windowSeconds ?? 20,
    head: analysis?.head?.windowSeconds ?? 20,
  };
  const incomingDuration = analysis?.head?.duration ?? incoming.duration ?? bounds.head * 2;
  const maxOverlap = Math.max(0.2, Math.min(12, bounds.tail, bounds.head, incomingDuration * 0.5));
  const maxCue = (seconds: number) => Math.max(0, Math.min(bounds.head - seconds, incomingDuration - seconds * 2));
  const update = (change: Partial<PlaylistTransition>) => {
    const next = { ...mix, ...change };
    next.seconds = Math.min(next.seconds, maxOverlap);
    next.trim = Math.min(next.trim, Math.max(0, bounds.tail - next.seconds));
    next.cue = Math.min(next.cue, maxCue(next.seconds));
    setDraft(next);
  };
  const save = async () => {
    setBusy(true);
    setError('');
    try {
      await changePlaylists({ type: 'setTransition', ...request, settings: draft });
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const listen = async () => {
    setStarting(true);
    setError('');
    try {
      const id = await preparePreview('Переход', `${outgoing.name} → ${incoming.name}`);
      if (!live.current) {
        closePreview(id);
        return;
      }
      previewId.current = id;
      const preset = mix.preset === 'auto' ? analysis?.suggested.preset ?? fallback.preset : mix.preset;
      await startPlaylistTransitionPreview(id, playlist.id, outgoing.id, incoming.id, { ...mix, preset });
    } catch (error) {
      if (previewId.current !== undefined) closePreview(previewId.current);
      if (live.current) setError(String(error));
    } finally {
      if (live.current) setStarting(false);
    }
  };

  return <div className='playlist-pair-editor'>
    {analyzing && <LoadingIndicator label='Анализ фрагментов треков' />}
    {analysisError && <p className='transition-status'>Волновые формы недоступны. Время можно задать вручную.</p>}
    <SwitchSettings label='Задать переход вручную' checked={draft !== null}
      onChange={(manual) => manual ? update({}) : setDraft(null)} />
    <div className='playlist-mix-waves'>
      <TransitionWaveform label='Уходит' title={outgoing.name} side='tail' data={analysis?.tail ?? null}
        overlap={mix.seconds} offset={mix.trim} onCue={draft ? (trim) => update({ trim }) : undefined} />
      <TransitionWaveform label='Вступает' title={incoming.name} side='head' data={analysis?.head ?? null}
        overlap={mix.seconds} offset={mix.cue} maxCue={maxCue(mix.seconds)}
        onCue={draft ? (cue) => update({ cue }) : undefined} />
    </div>
    {draft ? <>
      <SelectSettings label='Пресет' options={transitionPresets} value={mix.preset}
        onChange={(preset) => update({ preset })} />
      <div className='playlist-mix-controls'>
        <MixSlider label='Длительность' value={mix.seconds} min={0.2} max={maxOverlap} onChange={(seconds) => update({ seconds })} />
        <MixSlider label='Выход до конца' value={mix.trim} max={Math.max(0, bounds.tail - mix.seconds)} onChange={(trim) => update({ trim })} />
        <MixSlider label='Вход от начала' value={mix.cue} max={maxCue(mix.seconds)} onChange={(cue) => update({ cue })} />
        <MixSlider label='Сила фильтра' value={mix.intensity * 100} max={100} onChange={(percent) => update({ intensity: percent / 100 })}
          format={(value) => `${Math.round(value)}%`} />
      </div>
    </> : <p className='transition-status'>{transitionPresets.find(({ value }) => value === mix.preset)?.label} · {transitionSeconds(mix.seconds)}</p>}
    {error && <p role='alert' className='kit-error'>{error}</p>}
    {previewing && nativePreview?.duration && <Progress label='Ход прослушивания перехода'
      value={nativePreview.position} max={nativePreview.duration} />}
    <div className='kit-dialog-actions'>
      <Button pending={starting} disabled={starting || analyzing} onClick={() => {
        if (previewing && nativePreview) closePreview(nativePreview.id);
        else void listen();
      }}>{previewing && !starting ? 'Остановить' : 'Прослушать переход'}</Button>
      <Button variant='primary' pending={busy} disabled={busy || !dirty} onClick={() => void save()}>Сохранить</Button>
    </div>
  </div>;
}

function MixSlider({ label, value, min = 0, max, onChange, format = transitionSeconds }: {
  label: string;
  value: number;
  min?: number;
  max: number;
  onChange: (value: number) => void;
  format?: (value: number) => string;
}) {
  return <div className='playlist-mix-slider'>
    <span>{label}</span><span>{format(value)}</span>
    <Slider label={label} value={value} min={min} max={max} step={0.1} onValueChange={onChange}
      onValueCommit={onChange} disabled={max <= min} />
  </div>;
}
