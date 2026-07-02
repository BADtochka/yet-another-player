const isWindows = () => navigator.userAgent.includes('Windows');

export const toByteArray = (value: unknown): Uint8Array => {
  if (value instanceof Uint8Array) {
    return value;
  }

  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }

  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }

  if (Array.isArray(value)) {
    return Uint8Array.from(value);
  }

  if (
    value &&
    typeof value === 'object' &&
    'type' in value &&
    (value as { type: unknown }).type === 'Buffer' &&
    Array.isArray((value as { data?: unknown }).data)
  ) {
    return Uint8Array.from((value as unknown as { data: number[] }).data);
  }

  if (typeof value === 'string') {
    return new TextEncoder().encode(value);
  }

  return new Uint8Array(0);
};

export const concatBytes = (left: Uint8Array, right: Uint8Array) => {
  if (left.length === 0) {
    return right;
  }

  if (right.length === 0) {
    return left;
  }

  const combined = new Uint8Array(left.length + right.length);
  combined.set(left);
  combined.set(right, left.length);

  return combined;
};

const decodeWith = (bytes: Uint8Array, encoding: string) => {
  try {
    return new TextDecoder(encoding).decode(bytes);
  } catch {
    return null;
  }
};

export const decodeShellOutput = (value: unknown) => {
  if (typeof value === 'string') {
    return value;
  }

  const bytes = toByteArray(value);

  if (bytes.length === 0) {
    return '';
  }

  const utf8 = new TextDecoder('utf-8', { fatal: false }).decode(bytes);

  if (!isWindows() || !utf8.includes('\uFFFD')) {
    return utf8;
  }

  return decodeWith(bytes, 'windows-1252') ?? decodeWith(bytes, 'cp866') ?? utf8;
};

export class ShellLineDecoder {
  private buffer = new Uint8Array(0);

  push(chunk: unknown, onLine: (line: string) => void) {
    this.buffer = concatBytes(this.buffer, toByteArray(chunk));

    while (true) {
      const newlineIndex = this.buffer.indexOf(0x0a);

      if (newlineIndex === -1) {
        break;
      }

      const lineBytes = this.buffer.slice(0, newlineIndex);
      this.buffer = this.buffer.slice(newlineIndex + 1);
      const line = decodeShellOutput(lineBytes).replace(/\r$/, '');

      if (line.length > 0) {
        onLine(line);
      }
    }
  }

  flush(onLine: (line: string) => void) {
    if (this.buffer.length === 0) {
      return;
    }

    const line = decodeShellOutput(this.buffer).replace(/\r$/, '');
    this.buffer = new Uint8Array(0);

    if (line.length > 0) {
      onLine(line);
    }
  }
}

export const getYtDlpShellOptions = () => ({
  encoding: 'raw' as const,
  env: {
    PYTHONUTF8: '1',
    PYTHONIOENCODING: 'utf-8',
  },
});
