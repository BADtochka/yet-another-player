import { create } from 'zustand';
import { Channel, invoke } from '@tauri-apps/api/core';
import { logError, logInfo } from '@/utils/appLogs';
import { checkYtDlp, clearYtDlpCheck, getYtDlpLocation, YtDlpError, ytDlpFailureMessage } from '@/utils/ytDlp';

export type ExternalTool = 'yt-dlp' | 'ffmpeg';

export type ExternalToolStatus = {
  tool: ExternalTool;
  installed: boolean;
  version: string | null;
  path: string | null;
  launchError?: string;
};

export type ExternalToolInstallProgress = {
  tool: ExternalTool;
  status: 'downloading' | 'extracting' | 'checking' | 'waitingForDownload' | 'publishing' | 'installed';
  downloaded: number;
  total: number | null;
};

type BundleProgress = Omit<ExternalToolInstallProgress, 'tool'>;
type FfmpegStatus = Omit<ExternalToolStatus, 'tool'>;

const getYtDlpStatus = async (fresh = false): Promise<ExternalToolStatus> => {
  let path: string;
  try {
    path = await getYtDlpLocation();
  } catch (error) {
    if (error instanceof YtDlpError && error.state === 'missing') {
      return { tool: 'yt-dlp', installed: false, version: null, path: null };
    }
    throw error;
  }

  const result = await checkYtDlp(fresh);
  return {
    tool: 'yt-dlp',
    installed: result.state !== 'missing',
    version: result.state === 'ready' ? result.version : null,
    launchError: result.state === 'ready' || result.state === 'missing' ? undefined : ytDlpFailureMessage[result.state],
    path,
  };
};

export const getExternalToolStatus = async (tool: ExternalTool, fresh = false): Promise<ExternalToolStatus> => {
  if (tool === 'yt-dlp') return getYtDlpStatus(fresh);
  return { tool, ...await invoke<FfmpegStatus>('check_ffmpeg') };
};

type ToolInstall = { id: string; progress?: ExternalToolInstallProgress; cancelling: boolean };

export const useToolInstalls = create<Partial<Record<ExternalTool, ToolInstall>>>(() => ({}));
let installSequence = 0;

export async function installExternalTool(tool: ExternalTool): Promise<ExternalToolStatus> {
  if (useToolInstalls.getState()[tool]) throw new Error('Установка уже выполняется.');
  const id = `${Date.now()}-${++installSequence}`;
  const job: ToolInstall = { id, cancelling: false };
  useToolInstalls.setState({ [tool]: job });

  const progress = new Channel<BundleProgress>((event) => {
    const active = useToolInstalls.getState()[tool];
    if (active?.id === id) useToolInstalls.setState({ [tool]: { ...active, progress: { ...event, tool } } });
  });

  logInfo('Installing external tool', { tool });
  try {
    if (tool === 'ffmpeg') return { tool, ...await invoke<FfmpegStatus>('install_ffmpeg', { id, onProgress: progress }) };
    const bundle = await invoke<BundleProgress>('install_yt_dlp_bundle', { id, onProgress: progress });
    progress.onmessage({ ...bundle, status: 'checking' });
    const status = await getYtDlpStatus(true);
    if (status.version) progress.onmessage({ ...bundle, status: 'installed' });
    return status;
  } catch (error) {
    if (error !== 'installCancelled') logError('Failed to install external tool', { tool, error: String(error) });
    throw error;
  } finally {
    useToolInstalls.setState({ [tool]: undefined });
  }
}

export async function cancelExternalToolInstall(tool: ExternalTool) {
  const job = useToolInstalls.getState()[tool];
  if (!job || job.cancelling) return;
  if (await invoke<boolean>('cancel_tool_install', { tool, id: job.id })) {
    const active = useToolInstalls.getState()[tool];
    if (active?.id === job.id) useToolInstalls.setState({ [tool]: { ...active, cancelling: true } });
  }
}

export async function removeExternalTool(tool: ExternalTool) {
  await invoke('remove_external_tool', { tool });
  if (tool === 'yt-dlp') clearYtDlpCheck();
  return getExternalToolStatus(tool);
}
