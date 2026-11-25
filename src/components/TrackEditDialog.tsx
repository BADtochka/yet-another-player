import { invoke } from '@tauri-apps/api/core';
import { useState } from 'preact/hooks';
import { Button } from '@/components/Button';
import { Dialog } from '@/components/Dialog';
import { Input } from '@/components/Input';
import { LyricsOffsetField, previewLyricsOffset, readOffset, savedLyricsOffset } from '@/components/LyricsOffsetControl';
import type { Track } from '@/types/Track';

export function TrackEditDialog({ track, onDismiss, onSaved }: {
  track: Track;
  onDismiss: () => void;
  onSaved?: () => void;
}) {
  const [initial] = useState({
    name: track.name,
    artists: track.artists.join('; '),
    album: track.album ?? '',
    albumArtist: track.albumArtist ?? '',
    releaseDate: track.releaseDate?.match(/^\d{4}(?:-\d{2}){0,2}/)?.[0] ?? String(track.year ?? ''),
    genres: (track.genres.length ? track.genres : [track.genre ?? '']).join('; '),
    trackNumber: String(track.trackNumber ?? ''),
    discNumber: String(track.discNumber ?? ''),
    offset: String(savedLyricsOffset(track, 'track')),
    clipOffset: String(savedLyricsOffset(track, 'clip')),
  });
  const [values, setValues] = useState(initial);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const dismiss = () => {
    previewLyricsOffset(track.path);
    previewLyricsOffset(track.path, undefined, 'clip');
    onDismiss();
  };
  const input = (key: keyof typeof values, label: string, type = 'text') => <label>
    {label}
    <Input value={values[key]} type={type} min={type === 'number' ? 1 : undefined} disabled={saving}
      maxLength={512} data-initial-focus={key === 'name' ? true : undefined}
      onInput={(event) => setValues((draft) => ({ ...draft, [key]: event.currentTarget.value }))} />
  </label>;
  const save = async (event: SubmitEvent) => {
    event.preventDefault();
    const offsetMs = readOffset(values.offset);
    const clipOffsetMs = readOffset(values.clipOffset);
    if (saving || !values.name.trim() || offsetMs === undefined || clipOffsetMs === undefined) return;
    const { offset, clipOffset, ...metadata } = values;
    const changed = (Object.keys(metadata) as (keyof typeof metadata)[]).filter((key) => initial[key] !== metadata[key]);
    const offsets = {
      offsetMs: offsetMs === readOffset(initial.offset) ? undefined : offsetMs,
      clipOffsetMs: clipOffsetMs === readOffset(initial.clipOffset) ? undefined : clipOffsetMs,
    };
    if (!changed.length && offsets.offsetMs === undefined && offsets.clipOffsetMs === undefined) {
      dismiss();
      return;
    }
    setSaving(true);
    setError(undefined);
    try {
      const list = (value: string) => value.split(';').map((item) => item.trim()).filter(Boolean);
      const { name, artists, album, albumArtist, releaseDate, genres, trackNumber, discNumber } = metadata;
      if (!changed.length) await invoke('library_set_lyrics_offset', { id: track.id, ...offsets });
      else await invoke('library_edit_track', { id: track.id, ...offsets, changed, fields: {
        name, artists: list(artists), album: album || null, albumArtist: albumArtist || null, releaseDate: releaseDate || null,
        genres: list(genres), trackNumber: trackNumber ? Number(trackNumber) : null, discNumber: discNumber ? Number(discNumber) : null,
      } });
      onSaved?.();
      onDismiss();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  return <Dialog open title='Редактирование трека' busy={saving} onDismiss={dismiss}>
    <form className='kit-dialog-form track-edit-form' onSubmit={(event) => void save(event)}>
      {input('name', 'Название')}
      {input('artists', 'Исполнители через точку с запятой')}
      {input('album', 'Альбом')}
      {input('albumArtist', 'Исполнитель альбома')}
      {input('releaseDate', 'Дата релиза')}
      {input('genres', 'Жанры через точку с запятой')}
      <div className='track-edit-numbers'>{input('trackNumber', 'Номер трека', 'number')}{input('discNumber', 'Номер диска', 'number')}</div>
      <div className='track-edit-offsets'>
        <LyricsOffsetField label='Смещение текста для трека, мс' value={values.offset} disabled={saving} onChange={(offset) => {
          setValues((draft) => ({ ...draft, offset }));
          previewLyricsOffset(track.path, readOffset(offset));
        }} />
        <LyricsOffsetField label='Смещение текста для клипа, мс' value={values.clipOffset} disabled={saving} onChange={(clipOffset) => {
          setValues((draft) => ({ ...draft, clipOffset }));
          previewLyricsOffset(track.path, readOffset(clipOffset), 'clip');
        }} />
      </div>
      <p>Плюс задерживает текст. Минус показывает его раньше.</p>
      {error && <p className='kit-error' role='alert'>{error}</p>}
      <div className='kit-dialog-actions'>
        <Button type='button' disabled={saving} onClick={dismiss}>Отмена</Button>
        <Button type='submit' variant='primary' pending={saving}
          disabled={saving || !values.name.trim() || readOffset(values.offset) === undefined || readOffset(values.clipOffset) === undefined}>Сохранить</Button>
      </div>
    </form>
  </Dialog>;
}
