import { join } from '@tauri-apps/api/path';
import { open, readDir, stat } from '@tauri-apps/plugin-fs';
import { type Track } from '@/types/Track';

const ID3_HEADER_SIZE = 10;
const METADATA_CHUNK_SIZE = 1024 * 1024;
const PARSE_CONCURRENCY = 12;
const SKIP_DIR_NAMES = new Set(['.thumbnails', '.Trash', '$RECYCLE.BIN', 'System Volume Information']);
const TEXT_FRAME_IDS = new Set(['TIT2', 'TPE1', 'TALB', 'TYER', 'TDRC', 'TLEN']);

const MPEG1_LAYER3_BITRATES = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
const MPEG2_LAYER3_BITRATES = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
const MPEG1_SAMPLE_RATES = [44100, 48000, 32000, 0];
const MPEG2_SAMPLE_RATES = [22050, 24000, 16000, 0];
const MPEG25_SAMPLE_RATES = [11025, 12000, 8000, 0];

const latin1Decoder = new TextDecoder('latin1');
const utf8Decoder = new TextDecoder('utf-8');
const utf16LeDecoder = new TextDecoder('utf-16le');
const utf16BeDecoder = new TextDecoder('utf-16be');

const splitArtists = (artist: string) =>
  artist
    .split(/[;,&]/)
    .map((value) => value.trim())
    .filter(Boolean);

const fallbackTrackName = (path: string) => {
  const fileName = path.split(/[\\/]/).pop() ?? 'Unknown track';
  const extensionIndex = fileName.lastIndexOf('.');

  return extensionIndex > 0 ? fileName.slice(0, extensionIndex) : fileName;
};

const syncSafeSize = (bytes: Uint8Array, offset: number) =>
  (bytes[offset] << 21) | (bytes[offset + 1] << 14) | (bytes[offset + 2] << 7) | bytes[offset + 3];

const uint32Size = (bytes: Uint8Array, offset: number) =>
  (bytes[offset] << 24) | (bytes[offset + 1] << 16) | (bytes[offset + 2] << 8) | bytes[offset + 3];

const trimText = (value: string) => value.replace(/\0+$/g, '').trim();

const decodeText = (bytes: Uint8Array) => {
  if (bytes.length === 0) {
    return '';
  }

  const encoding = bytes[0];
  const payload = bytes.slice(1);

  if (encoding === 1) {
    if (payload[0] === 0xfe && payload[1] === 0xff) {
      return trimText(utf16BeDecoder.decode(payload.slice(2)));
    }

    if (payload[0] === 0xff && payload[1] === 0xfe) {
      return trimText(utf16LeDecoder.decode(payload.slice(2)));
    }

    return trimText(utf16LeDecoder.decode(payload));
  }

  if (encoding === 2) {
    return trimText(utf16BeDecoder.decode(payload));
  }

  if (encoding === 3) {
    return trimText(utf8Decoder.decode(payload));
  }

  return trimText(latin1Decoder.decode(payload));
};

const textTerminatorLength = (encoding: number) => (encoding === 1 || encoding === 2 ? 2 : 1);

const findTerminator = (bytes: Uint8Array, offset: number, encoding: number) => {
  const terminatorLength = textTerminatorLength(encoding);

  for (let index = offset; index <= bytes.length - terminatorLength; index += terminatorLength) {
    if (terminatorLength === 2) {
      if (bytes[index] === 0 && bytes[index + 1] === 0) {
        return index;
      }

      continue;
    }

    if (bytes[index] === 0) {
      return index;
    }
  }

  return -1;
};

const bytesToBase64 = (bytes: Uint8Array) => {
  let binary = '';

  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.slice(index, index + 0x8000));
  }

  return btoa(binary);
};

const parseCover = (bytes: Uint8Array) => {
  if (bytes.length < 4) {
    return null;
  }

  const encoding = bytes[0];
  let offset = 1;
  const mimeEnd = findTerminator(bytes, offset, 0);

  if (mimeEnd === -1) {
    return null;
  }

  const mimeType = latin1Decoder.decode(bytes.slice(offset, mimeEnd)) || 'image/jpeg';
  offset = mimeEnd + 1;

  if (offset >= bytes.length) {
    return null;
  }

  offset += 1;
  const descriptionEnd = findTerminator(bytes, offset, encoding);
  if (descriptionEnd === -1) {
    return null;
  }

  offset = descriptionEnd + textTerminatorLength(encoding);
  const image = bytes.slice(offset);

  return image.length > 0 ? `data:${mimeType};base64,${bytesToBase64(image)}` : null;
};

const id3TagEnd = (bytes: Uint8Array) => {
  if (bytes.length < ID3_HEADER_SIZE || bytes[0] !== 0x49 || bytes[1] !== 0x44 || bytes[2] !== 0x33) {
    return 0;
  }

  return ID3_HEADER_SIZE + syncSafeSize(bytes, 6);
};

const readMp3FrameHeader = (bytes: Uint8Array, offset: number) => {
  if (offset + 4 >= bytes.length || bytes[offset] !== 0xff || (bytes[offset + 1] & 0xe0) !== 0xe0) {
    return null;
  }

  const versionBits = (bytes[offset + 1] >> 3) & 0x03;
  const layerBits = (bytes[offset + 1] >> 1) & 0x03;
  if (layerBits !== 1) {
    return null;
  }

  const bitrateIndex = (bytes[offset + 2] >> 4) & 0x0f;
  const sampleRateIndex = (bytes[offset + 2] >> 2) & 0x03;
  const padding = (bytes[offset + 2] >> 1) & 0x01;

  const isMpeg1 = versionBits === 3;
  const isMpeg2 = versionBits === 2;
  const bitrates = isMpeg1 ? MPEG1_LAYER3_BITRATES : MPEG2_LAYER3_BITRATES;
  const sampleRates = isMpeg1
    ? MPEG1_SAMPLE_RATES
    : versionBits === 0
      ? MPEG25_SAMPLE_RATES
      : MPEG2_SAMPLE_RATES;

  const bitrateKbps = bitrates[bitrateIndex] ?? 0;
  const frameSampleRate = sampleRates[sampleRateIndex] ?? 0;
  if (bitrateKbps === 0 || frameSampleRate === 0) {
    return null;
  }

  const frameLength = Math.floor((144000 * bitrateKbps) / frameSampleRate) + padding;
  if (frameLength <= 0) {
    return null;
  }

  return {
    bitrateKbps,
    frameLength,
    frameSampleRate,
    isMpeg1,
    isMpeg2,
  };
};

const findFirstMp3Frame = (bytes: Uint8Array) => {
  let offset = id3TagEnd(bytes);

  while (offset + 4 < bytes.length) {
    const frame = readMp3FrameHeader(bytes, offset);
    if (frame) {
      return { offset, ...frame };
    }

    offset += 1;
  }

  return null;
};

const estimateMp3Duration = (bytes: Uint8Array, fileSize?: number) => {
  const firstFrame = findFirstMp3Frame(bytes);
  if (!firstFrame) {
    return null;
  }

  if (fileSize && fileSize > firstFrame.offset) {
    const audioBytes = fileSize - firstFrame.offset;
    return (audioBytes * 8) / (firstFrame.bitrateKbps * 1000);
  }

  let offset = firstFrame.offset;
  let totalSamples = 0;
  let sampleRate = firstFrame.frameSampleRate;

  while (offset + 4 < bytes.length) {
    const frame = readMp3FrameHeader(bytes, offset);
    if (!frame) {
      offset += 1;
      continue;
    }

    if (offset + frame.frameLength > bytes.length) {
      break;
    }

    sampleRate = frame.frameSampleRate;
    totalSamples += frame.isMpeg1 || frame.isMpeg2 ? 1152 : 576;
    offset += frame.frameLength;
  }

  if (sampleRate === 0 || totalSamples === 0) {
    return null;
  }

  return totalSamples / sampleRate;
};

const parseId3Tags = (bytes: Uint8Array, path: string): Track => {
  const track: Track = {
    name: fallbackTrackName(path),
    artists: [],
    lyrics: [],
    coverBase64: '',
    path,
    album: null,
    year: null,
    duration: null,
  };

  if (bytes.length < ID3_HEADER_SIZE || bytes[0] !== 0x49 || bytes[1] !== 0x44 || bytes[2] !== 0x33) {
    return track;
  }

  const version = bytes[3];
  const tagEnd = ID3_HEADER_SIZE + syncSafeSize(bytes, 6);
  let offset = ID3_HEADER_SIZE;

  while (offset + 10 <= bytes.length && offset + 10 <= tagEnd) {
    const frameId = latin1Decoder.decode(bytes.slice(offset, offset + 4));
    const frameSize = version === 4 ? syncSafeSize(bytes, offset + 4) : uint32Size(bytes, offset + 4);

    if (!/^[A-Z0-9]{4}$/.test(frameId) || frameSize <= 0) {
      break;
    }

    const frameStart = offset + 10;
    const frameEnd = frameStart + frameSize;
    if (frameEnd > bytes.length || frameEnd > tagEnd) {
      break;
    }

    const frame = bytes.slice(frameStart, frameEnd);

    if (TEXT_FRAME_IDS.has(frameId)) {
      const value = decodeText(frame);

      if (frameId === 'TIT2' && value) track.name = value;
      if (frameId === 'TPE1') track.artists = value ? splitArtists(value) : [];
      if (frameId === 'TALB') track.album = value || null;
      if (frameId === 'TYER' || frameId === 'TDRC') track.year = Number.parseInt(value.slice(0, 4), 10) || null;
      if (frameId === 'TLEN') {
        const milliseconds = Number.parseInt(value, 10);
        track.duration = Number.isFinite(milliseconds) && milliseconds > 0 ? milliseconds / 1000 : null;
      }
    }

    if (frameId === 'APIC' && !track.coverBase64) {
      track.coverBase64 = parseCover(frame) ?? '';
    }

    offset = frameEnd;
  }

  return track;
};

const readMetadataChunk = async (path: string) => {
  const file = await open(path, { read: true });

  try {
    const buffer = new Uint8Array(METADATA_CHUNK_SIZE);
    const bytesRead = await file.read(buffer);
    return buffer.subarray(0, bytesRead ?? 0);
  } finally {
    await file.close();
  }
};

const trackFromPath = async (path: string): Promise<Track> => {
  const [bytes, fileInfo] = await Promise.all([readMetadataChunk(path), stat(path)]);
  const track = parseId3Tags(bytes, path);

  if (track.duration === null) {
    track.duration = estimateMp3Duration(bytes, fileInfo.size ?? undefined);
  }

  return track;
};

const shouldSkipDirectory = (name: string) => SKIP_DIR_NAMES.has(name);

const collectMp3Paths = async (root: string) => {
  const paths: string[] = [];
  const directories = [root];

  while (directories.length > 0) {
    const directory = directories.pop();
    if (!directory) {
      continue;
    }

    let entries;

    try {
      entries = await readDir(directory);
    } catch (error) {
      console.warn('Skipped directory', directory, error);
      continue;
    }

    for (const entry of entries) {
      if (entry.name.startsWith('.')) {
        continue;
      }

      const path = await join(directory, entry.name);

      if (entry.isDirectory) {
        if (shouldSkipDirectory(entry.name)) {
          continue;
        }

        directories.push(path);
        continue;
      }

      if (entry.isFile && entry.name.toLowerCase().endsWith('.mp3')) {
        paths.push(path);
      }
    }
  }

  return paths;
};

const parseTracks = async (paths: string[]) => {
  const tracks: Track[] = [];

  for (let index = 0; index < paths.length; index += PARSE_CONCURRENCY) {
    const batch = paths.slice(index, index + PARSE_CONCURRENCY);
    const results = await Promise.allSettled(batch.map((path) => trackFromPath(path)));

    for (const result of results) {
      if (result.status === 'fulfilled') {
        tracks.push(result.value);
        continue;
      }

      console.warn('Failed to parse track', result.reason);
    }
  }

  return tracks;
};

export const getTracks = async (path: string) => {
  const paths = await collectMp3Paths(path);
  const tracks = await parseTracks(paths);
  tracks.sort((a, b) => a.path.localeCompare(b.path));

  return tracks;
};
