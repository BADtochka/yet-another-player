import { appDataDir, join } from '@tauri-apps/api/path';
import { exists, mkdir, open, remove, rename, writeFile } from '@tauri-apps/plugin-fs';
import { fetch } from '@tauri-apps/plugin-http';
import { Command } from '@tauri-apps/plugin-shell';
import JSZip from 'jszip';
import { logError, logInfo } from '@/utils/appLogs';

export type ExternalTool = 'yt-dlp' | 'ffmpeg';

export type ExternalToolStatus = {
  tool: ExternalTool;
  installed: boolean;
  version: string | null;
  path: string | null;
};

export type ExternalToolInstallProgress = {
  tool: ExternalTool;
  status: 'downloading' | 'installed';
  downloaded: number;
  total: number | null;
};

const YT_DLP = 'yt-dlp';
const FFMPEG = 'ffmpeg';

const isWindows = () => navigator.userAgent.includes('Windows');
const isMac = () => navigator.userAgent.includes('Mac');
const isLinux = () => navigator.userAgent.includes('Linux');
const isArm = () => navigator.userAgent.includes('aarch64') || navigator.userAgent.includes('arm64');

export const getYtDlpCommandName = () => (isWindows() ? 'yt-dlp.exe' : 'yt-dlp');

export const getFfmpegCommandName = () => (isWindows() ? 'ffmpeg.exe' : 'ffmpeg');

const getYtDlpFileName = () => {
  if (isWindows()) {
    return 'yt-dlp.exe';
  }

  return 'yt-dlp';
};

const getYtDlpDownloadUrl = () => {
  if (isWindows()) {
    return isArm()
      ? 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_arm64.exe'
      : 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe';
  }

  if (isMac()) {
    return 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos';
  }

  if (isLinux()) {
    return isArm()
      ? 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64'
      : 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux';
  }

  throw new Error('Unsupported operating system for yt-dlp auto-install');
};

const assertSupportedTool = (tool: ExternalTool) => {
  if (tool !== YT_DLP && tool !== FFMPEG) {
    throw new Error(`Unsupported external tool: ${tool}`);
  }
};

const getExternalToolsDir = async () => join(await appDataDir(), 'tools');

const getFfmpegDir = async () => join(await getExternalToolsDir(), 'ffmpeg');
const getFfmpegStagingDir = async () => join(await getExternalToolsDir(), 'ffmpeg-staging');

export const getFfmpegLocation = async () => getFfmpegDir();

const getFfmpegDownloadUrl = () => {
  if (isWindows()) {
    return 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip';
  }

  if (isLinux()) {
    return isArm()
      ? 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz'
      : 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz';
  }

  throw new Error('Unsupported operating system for ffmpeg auto-install');
};

export const getExternalToolPath = async (tool: ExternalTool) => {
  assertSupportedTool(tool);

  if (tool === FFMPEG) {
    return join(await getFfmpegDir(), isWindows() ? 'ffmpeg.exe' : 'ffmpeg');
  }

  return join(await getExternalToolsDir(), getYtDlpFileName());
};

export const ensureExternalToolExecutable = async (tool: ExternalTool) => {
  assertSupportedTool(tool);

  if (isWindows()) {
    return;
  }

  const path = await getExternalToolPath(tool);
  await Command.create('chmod', ['+x', path]).execute();

  if (tool === FFMPEG) {
    const ffprobePath = await join(await getFfmpegDir(), 'ffprobe');
    await Command.create('chmod', ['+x', ffprobePath]).execute();
  }
};

export const getExternalToolStatus = async (tool: ExternalTool): Promise<ExternalToolStatus> => {
  const path = await getExternalToolPath(tool);
  const installed = await exists(path);
  let version: string | null = null;

  if (installed) {
    try {
      const commandName = tool === FFMPEG ? getFfmpegCommandName() : getYtDlpCommandName();
      const output = await Command.create(commandName, [tool === FFMPEG ? '-version' : '--version']).execute();
      version = output.code === 0 ? output.stdout.trim() || null : null;
    } catch {
      version = null;
    }
  }

  return {
    tool,
    installed,
    version,
    path,
  };
};

const downloadBytes = async (
  tool: ExternalTool,
  url: string,
  onProgress?: (progress: ExternalToolInstallProgress) => void,
) => {
  const response = await fetch(url);

  if (!response.ok) {
    throw new Error(`Failed to download ${tool}: ${response.status} ${response.statusText}`);
  }

  const totalHeader = response.headers.get('content-length');
  const total = totalHeader ? Number.parseInt(totalHeader, 10) || null : null;
  const chunks: Uint8Array[] = [];
  let downloaded = 0;

  onProgress?.({ tool, status: 'downloading', downloaded, total });

  if (response.body) {
    const reader = response.body.getReader();

    while (true) {
      const { done, value } = await reader.read();

      if (done) {
        break;
      }

      chunks.push(value);
      downloaded += value.byteLength;
      onProgress?.({ tool, status: 'downloading', downloaded, total });
    }
  } else {
    const data = new Uint8Array(await response.arrayBuffer());
    chunks.push(data);
    downloaded = data.byteLength;
    onProgress?.({ tool, status: 'downloading', downloaded, total });
  }

  const data = new Uint8Array(downloaded);
  let offset = 0;

  for (const chunk of chunks) {
    data.set(chunk, offset);
    offset += chunk.byteLength;
  }

  return data;
};

const downloadFile = async (
  tool: ExternalTool,
  url: string,
  path: string,
  onProgress?: (progress: ExternalToolInstallProgress) => void,
) => {
  const response = await fetch(url);

  if (!response.ok) {
    throw new Error(`Failed to download ${tool}: ${response.status} ${response.statusText}`);
  }

  const totalHeader = response.headers.get('content-length');
  const total = totalHeader ? Number.parseInt(totalHeader, 10) || null : null;
  let downloaded = 0;

  onProgress?.({ tool, status: 'downloading', downloaded, total });

  const file = await open(path, {
    create: true,
    truncate: true,
    write: true,
  });

  try {
    if (response.body) {
      const reader = response.body.getReader();

      while (true) {
        const { done, value } = await reader.read();

        if (done) {
          break;
        }

        await file.write(value);
        downloaded += value.byteLength;
        onProgress?.({ tool, status: 'downloading', downloaded, total });
      }
    } else {
      const data = new Uint8Array(await response.arrayBuffer());
      await file.write(data);
      downloaded = data.byteLength;
      onProgress?.({ tool, status: 'downloading', downloaded, total });
    }
  } finally {
    await file.close();
  }

  return {
    downloaded,
    total,
  };
};

const installYtDlp = async (
  onProgress?: (progress: ExternalToolInstallProgress) => void,
): Promise<ExternalToolStatus> => {
  logInfo('Installing yt-dlp');
  const data = await downloadBytes(YT_DLP, getYtDlpDownloadUrl(), onProgress);
  const toolsDir = await getExternalToolsDir();
  await mkdir(toolsDir, { recursive: true });

  const path = await getExternalToolPath(YT_DLP);
  await writeFile(path, data);
  await ensureExternalToolExecutable(YT_DLP);

  onProgress?.({ tool: YT_DLP, status: 'installed', downloaded: data.byteLength, total: data.byteLength });
  logInfo('yt-dlp installed');

  return getExternalToolStatus(YT_DLP);
};

const getFfmpegArchiveExtension = () => (isWindows() ? 'zip' : 'tar.xz');

const getFfmpegBinaryNames = () => ({
  ffmpeg: isWindows() ? 'ffmpeg.exe' : 'ffmpeg',
  ffprobe: isWindows() ? 'ffprobe.exe' : 'ffprobe',
});

const findArchiveMember = (listing: string, fileName: string) =>
  listing
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.endsWith(`/bin/${fileName}`) || line.endsWith(`bin/${fileName}`));

const getStripComponents = (member: string) => {
  const parts = member.replace(/^\.\//, '').split('/');
  const binIndex = parts.lastIndexOf('bin');

  return binIndex === -1 ? parts.length - 1 : binIndex + 1;
};

const extractArchiveBinary = async (
  archivePath: string,
  stagingDir: string,
  member: string,
  destinationPath: string,
) => {
  const normalizedMember = member.replace(/^\.\//, '');
  const fileName = normalizedMember.split('/').at(-1);

  if (!fileName) {
    throw new Error(`Не удалось определить имя файла в архиве: ${member}`);
  }

  const output = await Command.create('tar', [
    '-xf',
    archivePath,
    '-C',
    stagingDir,
    `--strip-components=${getStripComponents(normalizedMember)}`,
    normalizedMember,
  ]).execute();

  if (output.code !== 0) {
    throw new Error(output.stderr.trim() || `Не удалось извлечь ${fileName}`);
  }

  await rename(await join(stagingDir, fileName), destinationPath);
};

const installFfmpegFromArchive = async (
  onProgress?: (progress: ExternalToolInstallProgress) => void,
): Promise<ExternalToolStatus> => {
  const { ffmpeg: ffmpegName, ffprobe: ffprobeName } = getFfmpegBinaryNames();
  const toolsDir = await getExternalToolsDir();
  const stagingDir = await getFfmpegStagingDir();
  const archivePath = await join(toolsDir, `ffmpeg.${getFfmpegArchiveExtension()}`);
  const ffmpegDir = await getFfmpegDir();

  logInfo('Installing ffmpeg from archive', { archivePath });

  await mkdir(toolsDir, { recursive: true });
  await mkdir(ffmpegDir, { recursive: true });

  if (await exists(stagingDir)) {
    await remove(stagingDir, { recursive: true });
  }

  await mkdir(stagingDir, { recursive: true });

  logInfo('Downloading ffmpeg archive');
  const download = await downloadFile(FFMPEG, getFfmpegDownloadUrl(), archivePath, onProgress);
  logInfo('ffmpeg archive downloaded', { bytes: download.downloaded });

  logInfo('Reading ffmpeg archive contents');
  const listOutput = await Command.create('tar', ['-tf', archivePath]).execute();

  if (listOutput.code !== 0) {
    throw new Error(listOutput.stderr.trim() || 'Не удалось прочитать содержимое архива ffmpeg');
  }

  const ffmpegMember = findArchiveMember(listOutput.stdout, ffmpegName);
  const ffprobeMember = findArchiveMember(listOutput.stdout, ffprobeName);

  if (!ffmpegMember || !ffprobeMember) {
    throw new Error(`ffmpeg archive does not contain ${ffmpegName} and ${ffprobeName}`);
  }

  logInfo('Extracting ffmpeg binaries');
  await extractArchiveBinary(archivePath, stagingDir, ffmpegMember, await join(ffmpegDir, ffmpegName));
  await extractArchiveBinary(archivePath, stagingDir, ffprobeMember, await join(ffmpegDir, ffprobeName));
  await remove(stagingDir, { recursive: true });
  await ensureExternalToolExecutable(FFMPEG);

  onProgress?.({ tool: FFMPEG, status: 'installed', downloaded: download.downloaded, total: download.total });
  logInfo('ffmpeg installed from archive');

  return getExternalToolStatus(FFMPEG);
};

const installFfmpegFromMacZip = async (
  onProgress?: (progress: ExternalToolInstallProgress) => void,
): Promise<ExternalToolStatus> => {
  logInfo('Installing ffmpeg from mac zip');
  const ffmpegUrl = 'https://evermeet.cx/ffmpeg/getrelease/zip';
  const ffprobeUrl = 'https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip';
  const ffmpegData = await downloadBytes(FFMPEG, ffmpegUrl, onProgress);
  const ffprobeData = await downloadBytes(FFMPEG, ffprobeUrl, onProgress);
  const ffmpegZip = await JSZip.loadAsync(ffmpegData);
  const ffprobeZip = await JSZip.loadAsync(ffprobeData);
  const ffmpegEntry = Object.values(ffmpegZip.files).find((entry) => entry.name.endsWith('ffmpeg'));
  const ffprobeEntry = Object.values(ffprobeZip.files).find((entry) => entry.name.endsWith('ffprobe'));

  if (!ffmpegEntry || !ffprobeEntry) {
    throw new Error('ffmpeg archive does not contain ffmpeg and ffprobe');
  }

  const ffmpegDir = await getFfmpegDir();
  await mkdir(ffmpegDir, { recursive: true });
  await writeFile(await join(ffmpegDir, 'ffmpeg'), await ffmpegEntry.async('uint8array'));
  await writeFile(await join(ffmpegDir, 'ffprobe'), await ffprobeEntry.async('uint8array'));
  await ensureExternalToolExecutable(FFMPEG);

  onProgress?.({
    tool: FFMPEG,
    status: 'installed',
    downloaded: ffmpegData.byteLength + ffprobeData.byteLength,
    total: ffmpegData.byteLength + ffprobeData.byteLength,
  });
  logInfo('ffmpeg installed from mac zip');

  return getExternalToolStatus(FFMPEG);
};

const installFfmpeg = (onProgress?: (progress: ExternalToolInstallProgress) => void) => {
  if (isMac()) {
    return installFfmpegFromMacZip(onProgress);
  }

  if (isWindows() || isLinux()) {
    return installFfmpegFromArchive(onProgress);
  }

  throw new Error('Unsupported operating system for ffmpeg auto-install');
};

export const installExternalTool = async (
  tool: ExternalTool,
  onProgress?: (progress: ExternalToolInstallProgress) => void,
): Promise<ExternalToolStatus> => {
  assertSupportedTool(tool);

  try {
    return await (tool === FFMPEG ? installFfmpeg(onProgress) : installYtDlp(onProgress));
  } catch (error) {
    logError(`Failed to install ${tool}`, error);
    throw error;
  }
};
