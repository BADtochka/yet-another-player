import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useId, useRef, useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Input } from '@/components/Input';
import { useOutsidePointer } from '@/hooks/useOutsidePointer';
import type { Track } from '@/types/Track';

export type LyricsTimeline = 'track' | 'clip';
type OffsetPreview = { path: string; offsetMs: number };

export const useLyricsOffsetPreview = create<Partial<Record<LyricsTimeline, OffsetPreview>>>(() => ({}));

export function previewLyricsOffset(path: string, offsetMs?: number, timeline: LyricsTimeline = 'track') {
  const preview = useLyricsOffsetPreview.getState()[timeline];
  if (offsetMs !== undefined) useLyricsOffsetPreview.setState({ [timeline]: { path, offsetMs } });
  else if (preview?.path === path) useLyricsOffsetPreview.setState({ [timeline]: undefined });
}

export function savedLyricsOffset(track: Track | null, timeline: LyricsTimeline) {
  return (timeline === 'clip' ? track?.clipLyricsOffsetMs : undefined) ?? track?.lyricsOffsetMs ?? 0;
}

const limit = 60_000;

export function readOffset(value: string) {
  const number = Number(value);
  return value.trim() && Number.isInteger(number) && Math.abs(number) <= limit ? number : undefined;
}

export function LyricsOffsetField({ value, onChange, disabled, label = 'Смещение текста, мс' }: {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  label?: string;
}) {
  const id = useId();
  const changeBy = (amount: number) => onChange(String(Math.max(-limit, Math.min(limit, (readOffset(value) ?? 0) + amount))));
  return <div className='lyrics-offset-field'>
    <label id={id + '-label'} htmlFor={id}>{label}</label>
    <div className='lyrics-offset-controls' role='group' aria-labelledby={id + '-label'}>
      <Button type='button' size='small' aria-label='Сдвинуть текст на 100 мс раньше' disabled={disabled}
        onClick={() => changeBy(-100)}>−</Button>
      <Input id={id} type='number' min={-limit} max={limit} step={1} value={value} disabled={disabled}
        onInput={(event) => onChange(event.currentTarget.value)} />
      <Button type='button' size='small' aria-label='Сдвинуть текст на 100 мс позже' disabled={disabled}
        onClick={() => changeBy(100)}>+</Button>
    </div>
  </div>;
}

export function LyricsOffsetControl({ track, onPreview }: { track: Track; onPreview: (value?: number) => void }) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState('0');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panelId = useId();

  const close = () => {
    if (saving) return;
    setOpen(false);
    onPreview();
  };
  useOutsidePointer(root, open, close);
  useEffect(() => () => onPreview(), [onPreview]);

  const save = async (event: SubmitEvent) => {
    event.preventDefault();
    const offsetMs = readOffset(value);
    if (saving || offsetMs === undefined) return;
    setSaving(true);
    setError(undefined);
    try {
      await invoke('library_set_lyrics_offset', { id: track.id, offsetMs });
      setOpen(false);
      trigger.current?.focus();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  return <div ref={root} className='lyrics-offset-popover'>
    <Button ref={trigger} type='button' size='small' icon='Time' aria-expanded={open} aria-controls={open ? panelId : undefined}
      onClick={() => {
        if (open) close();
        else { setValue(String(savedLyricsOffset(track, 'track'))); setError(undefined); setOpen(true); }
      }}>Смещение</Button>
    {open && <form id={panelId} className='lyrics-offset-panel' onSubmit={(event) => void save(event)}
      onKeyDown={(event) => {
        if (event.key === 'Escape') { event.stopPropagation(); close(); trigger.current?.focus(); }
      }}>
      <LyricsOffsetField value={value} disabled={saving} onChange={(value) => { setValue(value); onPreview(readOffset(value)); }} />
      <p>Плюс задерживает текст. Минус показывает его раньше.</p>
      {error && <p className='kit-error' role='alert'>{error}</p>}
      <div className='kit-dialog-actions'>
        <Button type='button' size='small' disabled={saving} onClick={close}>Отмена</Button>
        <Button type='submit' size='small' variant='primary' pending={saving} disabled={saving || readOffset(value) === undefined}>Сохранить</Button>
      </div>
    </form>}
  </div>;
}
