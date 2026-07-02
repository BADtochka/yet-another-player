import { join } from '@tauri-apps/api/path';
import { Command } from '@tauri-apps/plugin-shell';
import {
  ensureExternalToolExecutable,
  getExternalToolStatus,
  getFfmpegLocation,
  getYtDlpCommandName,
} from '@/utils/externalTools';

export type MusicSearchResult = {
  id: string;
  title: string;
  artist: string | null;
  duration: number | null;
  thumbnail: string | null;
  webpageUrl: string;
};

export type MusicDownloadProgress = {
  percent: number | null;
  status: string;
};

const SEARCH_LIMIT = 8;

type YtDlpSearchItem = {
  id?: string;
  title?: string;
  uploader?: string;
  channel?: string;
  channel_id?: string;
  duration?: number;
  thumbnail?: string;
  thumbnails?: Array<{ url?: string; width?: number; height?: number }>;
  webpage_url?: string;
  url?: string;
};

const parseJsonLines = (stdout: string) =>
  stdout
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line) as YtDlpSearchItem);

const bestThumbnail = (item: YtDlpSearchItem) => {
  if (item.thumbnail) {
    return item.thumbnail;
  }

  return item.thumbnails
    ?.filter((thumbnail) => thumbnail.url)
    .sort((a, b) => (b.width ?? 0) * (b.height ?? 0) - (a.width ?? 0) * (a.height ?? 0))[0]
    ?.url ?? null;
};

const normalizeResult = (item: YtDlpSearchItem): MusicSearchResult | null => {
  const id = item.id?.trim();
  const title = item.title?.trim();
  const webpageUrl = item.webpage_url?.trim() || (id ? `https://www.youtube.com/watch?v=${id}` : item.url?.trim());

  if (!id || !title || !webpageUrl) {
    return null;
  }

  return {
    id,
    title,
    artist: item.uploader?.trim() || item.channel?.trim() || null,
    duration: typeof item.duration === 'number' && Number.isFinite(item.duration) ? item.duration : null,
    thumbnail: bestThumbnail(item),
    webpageUrl,
  };
};

const ensureYtDlpInstalled = async () => {
  const status = await getExternalToolStatus('yt-dlp');

  if (!status.installed) {
    throw new Error('yt-dlp не установлен');
  }

  await ensureExternalToolExecutable('yt-dlp');
};

const normalizeYtDlpError = (stderr: string) => {
  if (stderr.includes('Sign in to confirm your age')) {
    return 'YouTube просит подтверждение возраста. Для этого источника нужны cookies браузера.';
  }

  return stderr.trim() || 'yt-dlp завершился с ошибкой';
};

const executeYtDlp = async (args: string[], options?: { allowPartialStdout?: boolean }) => {
  await ensureYtDlpInstalled();

  const output = await Command.create(getYtDlpCommandName(), args).execute();

  if (output.code !== 0) {
    if (options?.allowPartialStdout && output.stdout.trim()) {
      return output.stdout;
    }

    throw new Error(normalizeYtDlpError(output.stderr));
  }

  return output.stdout;
};

const parseDownloadProgress = (line: string): MusicDownloadProgress | null => {
  const percentMatch = line.match(/\[download\]\s+(\d+(?:\.\d+)?)%/);

  if (percentMatch) {
    return {
      percent: Number.parseFloat(percentMatch[1]),
      status: line.trim(),
    };
  }

  if (line.includes('[download] Destination:') || line.includes('[download] 100%')) {
    return {
      percent: line.includes('100%') ? 100 : null,
      status: line.trim(),
    };
  }

  return null;
};

const parseDownloadedPath = (line: string) => {
  const destinationMatch = line.match(/\[(?:download|ExtractAudio)\] Destination: (.+)$/);

  if (destinationMatch) {
    return destinationMatch[1].trim();
  }

  const existingFileMatch = line.match(/\[download\] (.+) has already been downloaded/);

  if (existingFileMatch) {
    return existingFileMatch[1].trim();
  }

  return null;
};

const executeYtDlpStreaming = async (
  args: string[],
  onProgress?: (progress: MusicDownloadProgress) => void,
) => {
  await ensureYtDlpInstalled();

  const command = Command.create(getYtDlpCommandName(), args);
  let stderr = '';
  let downloadedPath: string | null = null;

  command.stdout.on('data', (chunk) => {
    for (const line of chunk.split('\n')) {
      const progress = parseDownloadProgress(line);
      const path = parseDownloadedPath(line);

      if (progress) {
        onProgress?.(progress);
      }

      if (path) {
        downloadedPath = path;
      }
    }
  });

  command.stderr.on('data', (chunk) => {
    stderr += chunk;
  });

  await new Promise<void>((resolve, reject) => {
    command.on('close', (data) => {
      if (data.code === 0) {
        resolve();
        return;
      }

      reject(new Error(normalizeYtDlpError(stderr)));
    });

    command.on('error', (error) => reject(new Error(error)));

    void command.spawn().catch((error) => reject(error instanceof Error ? error : new Error(String(error))));
  });

  return downloadedPath;
};

export const searchMusic = async (query: string) => {
  const normalizedQuery = query.trim();

  if (normalizedQuery.length < 2) {
    return [];
  }

  const stdout = await executeYtDlp([
    '--dump-json',
    '--flat-playlist',
    '--skip-download',
    '--no-warnings',
    '--playlist-end',
    String(SEARCH_LIMIT),
    `ytsearch${SEARCH_LIMIT}:${normalizedQuery}`,
  ], { allowPartialStdout: true });

  return parseJsonLines(stdout)
    .map(normalizeResult)
    .filter((result): result is MusicSearchResult => result !== null);
};

export const getMusicPreviewUrl = async (result: MusicSearchResult) => {
  const stdout = await executeYtDlp([
    '--get-url',
    '--no-playlist',
    '--no-warnings',
    '--format',
    'bestaudio/best',
    result.webpageUrl,
  ]);

  return stdout
    .split('\n')
    .map((line) => line.trim())
    .find(Boolean) ?? null;
};

export const downloadMusic = async (
  result: MusicSearchResult,
  targetFolder: string,
  onProgress?: (progress: MusicDownloadProgress) => void,
) => {
  const outputTemplate = await join(targetFolder, '%(title).200B [%(id)s].%(ext)s');
  const ffmpegStatus = await getExternalToolStatus('ffmpeg');
  const ffmpegArgs = ffmpegStatus.installed
    ? ['--extract-audio', '--audio-format', 'mp3', '--audio-quality', '0', '--embed-thumbnail', '--add-metadata', '--ffmpeg-location', await getFfmpegLocation()]
    : ['--format', 'bestaudio[ext=m4a]/bestaudio[ext=webm]/bestaudio/best'];

  onProgress?.({ percent: 0, status: 'Подготовка...' });

  const downloadedPath = await executeYtDlpStreaming([
    ...ffmpegArgs,
    '--no-playlist',
    '--no-warnings',
    '--newline',
    '-o',
    outputTemplate,
    result.webpageUrl,
  ], onProgress);

  onProgress?.({ percent: 100, status: 'Готово' });

  return downloadedPath;
};
