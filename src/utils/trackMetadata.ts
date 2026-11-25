import type { Track } from '@/types/Track';

const yearMonth = new Intl.DateTimeFormat('ru', { year: 'numeric', month: 'long', timeZone: 'UTC' });
const fullDate = new Intl.DateTimeFormat('ru', { year: 'numeric', month: 'long', day: 'numeric', timeZone: 'UTC' });
const number = new Intl.NumberFormat('ru');
const plural = new Intl.PluralRules('ru');
const playForms = { one: 'воспроизведение', few: 'воспроизведения', many: 'воспроизведений', other: 'воспроизведения' };

function releaseDate(track: Pick<Track, 'releaseDate' | 'year'>) {
  const match = /^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?(?=$|[T ])/u.exec(track.releaseDate ?? '');
  const year = Number(match?.[1] ?? track.year);
  if (!year) return undefined;
  const month = Number(match?.[2]);
  if (month < 1 || month > 12 || !Number.isFinite(month)) return { precision: 1, label: String(year) };
  const day = Number(match?.[3]);
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day >= 1 && day <= 31 ? day : 1);
  const hasDay = date.getUTCDate() === day && date.getUTCMonth() === month - 1;
  if (!hasDay) date.setUTCFullYear(year, month - 1, 1);
  const label = (hasDay ? fullDate : yearMonth).format(date).replace(/\sг\.$/u, '');
  return { precision: hasDay ? 3 : 2, label };
}

export function formatReleaseDate(track: Track) {
  return releaseDate(track)?.label;
}

export function formatPlayCount(count: number) {
  const form = plural.select(count);
  return `${number.format(count)} ${playForms[form as keyof typeof playForms] ?? playForms.other}`;
}

export function describePlayback(tracks: Track[], release?: { date: string | null; genres?: string[] }) {
  let date = release ? releaseDate({ releaseDate: release.date, year: null }) : undefined;
  let count = 0;
  const genres = new Map(release?.genres?.map((genre) => [genre.toLowerCase(), genre]));
  for (const track of tracks) {
    const candidate = releaseDate(track);
    if (candidate && (!date || candidate.precision > date.precision)) date = candidate;
    count += track.playCount;
    for (const value of track.genres.length ? track.genres : [track.genre ?? '']) {
      for (const part of value.split(/[,;]/u)) {
        const genre = part.trim();
        const key = genre.toLowerCase();
        if (genre && !genres.has(key)) genres.set(key, genre);
      }
    }
  }
  return [
    ...(date ? [{ icon: 'Calendar' as const, label: date.label }] : []),
    { icon: 'Time' as const, label: formatPlayCount(count) },
    ...(genres.size ? [{ icon: 'Melody' as const, label: [...genres.values()].join(', ') }] : []),
  ];
}
